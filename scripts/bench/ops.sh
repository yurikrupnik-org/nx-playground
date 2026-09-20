#!/usr/bin/env bash
# Per-operation wire cost — the bytes one API call actually moves.
#
#   scripts/bench/ops.sh [--base URL] [--runs N] [--keep]
#
# This is the number that distinguishes a CLI/TUI from a browser app. A
# browser pays the first-render asset closure once (`scripts/bench/assets.sh`)
# and then the same JSON per operation; a CLI/TUI pays no asset payload at all
# and only ever moves this. Same API, same rows, same basis.
#
# Measured per operation, over N runs, reported as the MEDIAN:
#   * request bytes  = every `=> Send header` / `=> Send data` byte count in
#     curl's own `--trace-ascii` log, i.e. the request line + headers + body
#     exactly as they went onto the socket. NOT `%{size_request}`: curl 8.7.1
#     (the macOS system curl) reports **0** for it, which would silently
#     under-report every request row.
#   * response bytes = `%{size_header}` (status line + headers) + body bytes
#   * body bytes are `wc -c` of what curl WROTE, not `%{size_download}`:
#     with `Accept-Encoding: gzip` and no `--compressed`, curl stores the
#     compressed bytes, which is what crossed the socket. The gzip rows also
#     assert `content-encoding: gzip` came back — a server that ignores the
#     header is reported as `identity` with a note rather than silently
#     charged the raw size as if it had compressed.
#   * no proxy, no tcpdump: curl's own trace and counters ARE the wire, and
#     the two encodings are two separate request sets against one endpoint.
#
# The write path POSTs a marker todo and DELETEs it again (pass `--keep` to
# leave it). The DELETE is measured too, because "one write" for a CLI is one
# request and a cleanup is one more.
#
# Requires a running API (`just docker-up && just migrate todo && just run
# todo-api`). Exits 1 with the start command if it is not reachable — unlike
# `assets.sh` there is nothing to measure without it.
#
# Invoked by `just bench-ops`.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

BASE="http://127.0.0.1:8080"
RUNS=5
KEEP=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base) BASE="${2:?--base needs a URL}"; shift 2 ;;
    --runs) RUNS="${2:?--runs needs a number}"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h | --help)
      sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//; $d'
      exit 0
      ;;
    *) echo "FATAL: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
done

BASE="${BASE%/}"
API="$BASE/api"

command -v curl >/dev/null 2>&1 || { echo "FATAL: curl not found in PATH." >&2; exit 1; }
command -v awk >/dev/null 2>&1 || { echo "FATAL: awk not found in PATH." >&2; exit 1; }

if ! curl -fsS -o /dev/null --max-time 5 "$API/todos"; then
  cat >&2 <<EOF
FATAL: no API answering GET $API/todos

  just docker-up && just migrate todo && just run todo-api

Point this elsewhere with --base URL (default http://127.0.0.1:8080).
EOF
  exit 1
fi

TMP="$(mktemp -d -t bench-ops)"
trap 'rm -rf "$TMP"' EXIT

POST_BODY="$TMP/post.json"
printf '{"title":"bench-ops wire cost","description":"measured by scripts/bench/ops.sh","priority":"high"}' >"$POST_BODY"
POST_BODY_BYTES="$(wc -c <"$POST_BODY" | tr -d ' ')"

median() { # median <n> [n ...]
  printf '%s\n' "$@" | sort -n | awk '
    { a[NR] = $1 }
    END {
      if (NR == 0) { printf "n/a"; exit }
      if (NR % 2) { printf "%d", a[(NR + 1) / 2] }
      else { printf "%d", (a[NR / 2] + a[NR / 2 + 1]) / 2 + 0.5 }
    }'
}

# One request -> "req_bytes|resp_bytes|status|encoding|body_bytes"
one_call() { # one_call <method> <url> <gzip:0|1> [body file]
  local method=$1 url=$2 want_gzip=$3 body=${4:-} args=() out enc
  args=(-sS -o "$TMP/body" -D "$TMP/headers" --trace-ascii "$TMP/trace" -X "$method" "$url")
  [[ "$want_gzip" == 1 ]] && args+=(-H 'Accept-Encoding: gzip')
  if [[ -n "$body" ]]; then
    args+=(-H 'Content-Type: application/json' --data-binary "@$body")
  fi
  # size_download is deliberately unused: curl reports the DEcompressed size
  # to its caller, and size_request reports 0 on curl 8.7.1 — hence the trace.
  out="$(curl "${args[@]}" -w '%{http_code} %{size_upload} %{size_header}')"
  local code upload header body_bytes req
  read -r code upload header <<<"$out"
  body_bytes="$(wc -c <"$TMP/body" | tr -d ' ')"
  # `=> Send header, 87 bytes (0x57)` / `=> Send data, 98 bytes (0x62)`
  req="$(awk '/^=> Send (header|data),/ { n += $4 } END { printf "%d", n + 0 }' "$TMP/trace")"
  # Fall back to curl's counters if a build ever stops emitting the trace.
  [[ "$req" -gt 0 ]] || req="$upload"
  enc=identity
  grep -qi '^content-encoding: *gzip' "$TMP/headers" && enc=gzip
  printf '%s|%s|%s|%s|%s' \
    "$req" "$((header + body_bytes))" "$code" "$enc" "$body_bytes"
}

# serde_json emits compact JSON, but never depend on that: tolerate spacing.
UUID_RE='.*"id" *: *"([0-9a-fA-F-]{36})".*'

created_id() { # created_id — POST one todo, print its uuid (or empty)
  local body
  body="$(curl -sS -X POST -H 'Content-Type: application/json' \
    --data-binary "@$POST_BODY" "$API/todos")" || return 0
  sed -nE "s/$UUID_RE/\\1/p" <<<"$body" | awk 'NR==1'
}

delete_id() { curl -sS -o /dev/null -X DELETE "$API/todos/$1" || true; }

ROWS=()   # op|encoding|req median|resp median|body median|status|note
IDS=()    # todos created by the write measurements

measure() { # measure <label> <method> <url> <gzip> [body file] [collect-id] [fresh-target]
  # `fresh-target`: mint a new todo before EVERY run and address it. A DELETE
  # is not repeatable against one id — run 1 gets 204 and the rest get 404, so
  # the median status published a 404 for a row labelled "DELETE". The URL
  # argument is ignored in this mode.
  local label=$1 method=$2 url=$3 want_gzip=$4 body=${5:-} del=${6:-0} fresh=${7:-0}
  local reqs=() resps=() bodies=() encs="" code="" i r rq rs enc bb
  for ((i = 0; i < RUNS; i++)); do
    if [[ "$fresh" == 1 ]]; then
      local fresh_id
      fresh_id="$(created_id)"
      [[ -n "$fresh_id" ]] || continue
      url="$API/todos/$fresh_id"
    fi
    r="$(one_call "$method" "$url" "$want_gzip" "$body")"
    IFS='|' read -r rq rs code enc bb <<<"$r"
    reqs+=("$rq")
    resps+=("$rs")
    bodies+=("$bb")
    encs="$enc"
    if [[ "$del" == 1 ]]; then
      local id
      id="$(sed -nE "s/$UUID_RE/\\1/p" "$TMP/body" | awk 'NR==1')"
      [[ -n "$id" ]] && IDS+=("$id")
    fi
  done
  local note=""
  if [[ "$want_gzip" == 1 && "$encs" != gzip ]]; then
    note="server answered \`identity\` — gzip requested but NOT negotiated, so these are the raw bytes"
  fi
  ROWS+=("$label|$encs|$(median "${reqs[@]}")|$(median "${resps[@]}")|$(median "${bodies[@]}")|$code|$note")
}

# --- read path --------------------------------------------------------------
# Row size scales with the list, so the row is only quotable with this count.
# `|| true`: grep exits 1 on an empty list and pipefail would kill the run.
LIST_COUNT="$(curl -sS "$API/todos" | grep -o '"id" *: *"' | wc -l | tr -d ' ' || true)"

measure "GET /api/todos" GET "$API/todos" 0
measure "GET /api/todos (Accept-Encoding: gzip)" GET "$API/todos" 1

# --- write path -------------------------------------------------------------
measure "POST /api/todos" POST "$API/todos" 0 "$POST_BODY" 1
measure "POST /api/todos (Accept-Encoding: gzip)" POST "$API/todos" 1 "$POST_BODY" 1

# One DELETE per run, each on a todo minted for exactly that run.
measure "DELETE /api/todos/{id}" DELETE "" 0 "" 0 1

# --- cleanup ----------------------------------------------------------------
CLEANED=0
if [[ "$KEEP" == 0 ]]; then
  for id in "${IDS[@]:-}"; do
    [[ -n "$id" ]] || continue
    delete_id "$id"
    CLEANED=$((CLEANED + 1))
  done
fi

# --- report -----------------------------------------------------------------
echo "# Per-operation wire cost — $API"
echo
echo "## $(date -u +%F) — median of $RUNS runs per row"
echo
echo "- Reproduce: \`just bench-ops\` (\`scripts/bench/ops.sh --base $BASE --runs $RUNS\`), $(date -u +%FT%TZ)."
echo "- \`GET /api/todos\` was measured against a list of **$LIST_COUNT** todos — this number scales with the table, so quote it with the row."
echo "- request bytes = request line + headers + body; response bytes = status line + headers + body as it crossed the socket (\`wc -c\` of what curl stored, so a gzip row is compressed bytes)."
echo "- A CLI/TUI pays only these bytes per command. A browser app pays them too, *after* the one-time first-render closure in \`just bench-assets\`."
echo
echo "| operation | content-encoding | request bytes | response bytes | of which body | status | note |"
echo "|---|---|---:|---:|---:|---:|---|"
for row in "${ROWS[@]}"; do
  IFS='|' read -r op enc req resp body code note <<<"$row"
  echo "| $op | $enc | $req | $resp | $body | $code | ${note:-} |"
done
echo
echo "- POST body on the wire: $POST_BODY_BYTES bytes of JSON (\`{\"title\",\"description\",\"priority\"}\`)."
if [[ "$KEEP" == 0 ]]; then
  echo "- Cleanup: deleted the $CLEANED todo(s) these measurements created (\`--keep\` leaves them)."
else
  echo "- \`--keep\`: ${#IDS[@]} measurement todo(s) left in the database."
fi
