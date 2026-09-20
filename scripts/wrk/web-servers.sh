#!/usr/bin/env bash
# Compare the static web server images (nginx / caddy / static-web-server)
# serving the SAME SPA dist, built from manifests/dockers/Dockerfile targets.
#
#   web-servers.sh test  [dist]                          behavior-parity checks
#   web-servers.sh bench [dist] [duration] [conns] [threads]   wrk + p50/75/90/99 table
#
# Works on both dist layouts: vite's `assets/index-<hash>.{js,css}` and
# trunk's root-level `<crate>-<hash>.js` + `<crate>_bg-<hash>.wasm`. When the
# dist contains a `.wasm`, it is asserted (content-type `application/wasm`,
# immutable caching, compression) and benchmarked as its own path — for a WASM
# arm that file, not the JS shim, is the payload.
#
# Invoked by `just test-web-servers` / `just bench-web-compare`.
set -euo pipefail

MODE="${1:-bench}"
DIST="${2:-apps/todo/web/dist}"
DURATION="${3:-10s}"
CONNS="${4:-64}"
THREADS="${5:-4}"

SERVERS=(nginx caddy static-web-server)
BASE_PORT=9180
DOCKERFILE=manifests/dockers/Dockerfile

cd "$(git rev-parse --show-toplevel)"

port_of() {
  local i=0 s
  for s in "${SERVERS[@]}"; do
    if [[ "$s" == "$1" ]]; then echo $((BASE_PORT + i)); return; fi
    i=$((i + 1))
  done
  echo "unknown server: $1" >&2
  return 1
}

cleanup() { # containers only: this also runs BEFORE the containers start
  local s
  for s in "${SERVERS[@]}"; do docker rm -f "web-cmp-$s" >/dev/null 2>&1 || true; done
}
TMPD="$(mktemp -d -t web-cmp)"
trap 'cleanup; rm -rf "$TMPD"' EXIT

# --- dist -------------------------------------------------------------------
DEFAULT_DIST=apps/todo/web/dist
if [[ ! -f "$DIST/index.html" ]]; then
  if [[ "$DIST" == "$DEFAULT_DIST" ]]; then
    echo "==> $DIST/index.html missing; building todo-web"
    bun nx run todo-web:build
  else
    echo "FATAL: no index.html in $DIST — build that dist first (trunk/vite);" >&2
    echo "       only $DEFAULT_DIST is auto-built here." >&2
    exit 1
  fi
fi
[[ -f "$DIST/index.html" ]] || { echo "FATAL: no index.html in $DIST" >&2; exit 1; }

# Asset discovery must cope with BOTH dist layouts. vite writes hashed files
# under `assets/`; trunk writes `<crate>-<hash>.js`, `<crate>_bg-<hash>.wasm`
# and `<crate>-<hash>.css` at the dist ROOT with no `assets/` dir at all — the
# old `find assets -name '*.js'` hard-failed there. Prefer whatever
# index.html actually references (that is what a first render fetches), then
# fall back to the first match in sorted order, so the pick is deterministic.
pick_asset() { # pick_asset <ext> -> dist-relative path, or empty
  local ext=$1 refs p
  refs="$(grep -oE "[\"'][^\"']+\.$ext([?#][^\"']*)?[\"']" "$DIST/index.html" 2>/dev/null |
    sed -E "s/^[\"']//; s/[\"']$//; s/[?#].*$//; s#^/##; s#^\./##" |
    grep -vE '^(https?:)?//|^data:' || true)"
  while IFS= read -r p; do
    if [[ -n "$p" && -f "$DIST/$p" ]]; then
      printf '%s' "$p"
      return 0
    fi
  done <<<"$refs"
  (cd "$DIST" && find . -type f -name "*.$ext" | sed 's#^\./##' | sort | awk 'NR==1')
}

ASSET="$(pick_asset js)"
[[ -n "$ASSET" ]] || {
  echo "FATAL: no .js asset in $DIST (checked index.html's references, then $DIST/**/*.js)" >&2
  exit 1
}
# A WASM dist is the only arm where the interesting asset is not JS, so it is
# benched and asserted as its own path when present.
WASM_ASSET="$(pick_asset wasm)"
echo "==> assets: js=$ASSET wasm=${WASM_ASSET:-none}"

# --- build + start ----------------------------------------------------------
echo "==> building images (dist: $DIST)"
for s in "${SERVERS[@]}"; do
  docker build --target "$s" --build-arg DIST_PATH="$DIST" \
    -f "$DOCKERFILE" -t "web-cmp-$s" . >/dev/null
done

echo "==> starting containers"
cleanup
for s in "${SERVERS[@]}"; do
  # UPSTREAM_* feeds the /api reverse proxy (nginx envsubst template requires
  # them; caddy has Caddyfile defaults; static-web-server ignores them).
  docker run -d --name "web-cmp-$s" -p "$(port_of "$s"):8080" \
    -e UPSTREAM_NAME=backend -e UPSTREAM_HOST=host.docker.internal -e UPSTREAM_PORT=8080 \
    "web-cmp-$s" >/dev/null
done

for s in "${SERVERS[@]}"; do
  p="$(port_of "$s")"
  ok=""
  for _ in $(seq 1 50); do
    if curl -fsS -o /dev/null "http://127.0.0.1:$p/health" 2>/dev/null; then ok=1; break; fi
    sleep 0.2
  done
  if [[ -z "$ok" ]]; then
    echo "FATAL: $s never became healthy on :$p" >&2
    docker logs "web-cmp-$s" >&2 || true
    exit 1
  fi
done

# --- test mode ---------------------------------------------------------------
FAILURES=0

expect() { # expect <server> <desc> <ok:0/1>
  if [[ "$3" == 0 ]]; then
    echo "  ok   $2"
  else
    echo "  FAIL $2"
    FAILURES=$((FAILURES + 1))
  fi
}

run_tests() {
  local s=$1 base code headers
  base="http://127.0.0.1:$(port_of "$s")"
  echo "== $s =="

  code=$(curl -s -o /dev/null -w '%{http_code}' "$base/health")
  expect "$s" "/health -> 200 (got $code)" "$([[ $code == 200 ]]; echo $?)"

  # Byte-identical to the dist's own index.html, rather than grepping for a
  # Solid-specific `<div id="root">` mount point: same check for every dist
  # layout (a trunk/Leptos index mounts elsewhere), and strictly stronger.
  curl -fsS -o "$TMPD/root" "$base/"
  expect "$s" "/ serves index.html byte-identical to $DIST/index.html" \
    "$(cmp -s "$TMPD/root" "$DIST/index.html"; echo $?)"

  # Measured: static-web-server's FALLBACK page drops index.html's trailing
  # newline (595 vs 596 bytes) while its `/` is byte-exact, so the fallback is
  # compared modulo trailing newline — still the whole document, not a grep
  # for one Solid-specific `<div id="root">` that a Leptos index lacks.
  curl -fsS -o "$TMPD/fallback" "$base/some/client/route"
  expect "$s" "SPA fallback serves index.html (modulo trailing newline)" \
    "$([[ "$(cat "$TMPD/fallback")" == "$(cat "$DIST/index.html")" ]]; echo $?)"

  headers=$(curl -fsS -D - -o /dev/null "$base/$ASSET")
  expect "$s" "asset $ASSET -> 200 + js content-type" \
    "$(grep -qi 'content-type:.*javascript' <<<"$headers"; echo $?)"
  expect "$s" "asset has long-lived cache-control" \
    "$(grep -qi 'cache-control:.*\(immutable\|max-age=[0-9]\{4,\}\)' <<<"$headers"; echo $?)"

  headers=$(curl -fsS -H 'Accept-Encoding: gzip' -D - -o /dev/null "$base/$ASSET")
  expect "$s" "asset served gzip-compressed" \
    "$(grep -qi 'content-encoding: gzip' <<<"$headers"; echo $?)"

  # A WASM dist's interesting asset is not JS. Same three rules as the JS
  # asset, asserted not weakened: if an image does not serve
  # `application/wasm`, does not cache it immutably, or refuses to compress
  # it, that is a finding about the image's config, not about this check.
  if [[ -n "$WASM_ASSET" ]]; then
    headers=$(curl -fsS -D - -o /dev/null "$base/$WASM_ASSET")
    expect "$s" "asset $WASM_ASSET -> 200 + application/wasm content-type" \
      "$(grep -qi 'content-type:.*application/wasm' <<<"$headers"; echo $?)"
    expect "$s" "wasm asset has long-lived cache-control" \
      "$(grep -qi 'cache-control:.*\(immutable\|max-age=[0-9]\{4,\}\)' <<<"$headers"; echo $?)"

    headers=$(curl -fsS -H 'Accept-Encoding: gzip' -D - -o /dev/null "$base/$WASM_ASSET")
    expect "$s" "wasm asset served gzip-compressed" \
      "$(grep -qi 'content-encoding: gzip' <<<"$headers"; echo $?)"
  fi
}

# --- bench mode ---------------------------------------------------------------
bench_one() { # bench_one <server> <path> -> "rps|p50|p75|p90|p99"
  local s=$1 path=$2 url
  url="http://127.0.0.1:$(port_of "$s")$path"
  # short warmup, discarded
  wrk -t2 -c16 -d2s "$url" >/dev/null
  wrk -t"$THREADS" -c"$CONNS" -d"$DURATION" --latency "$url" |
    awk '/Requests\/sec/{rps=$2}
         $1=="50%"{p50=$2} $1=="75%"{p75=$2} $1=="90%"{p90=$2} $1=="99%"{p99=$2}
         END{printf "%s|%s|%s|%s|%s", rps, p50, p75, p90, p99}'
}

case "$MODE" in
  test)
    for s in "${SERVERS[@]}"; do run_tests "$s"; done
    echo
    if [[ $FAILURES -gt 0 ]]; then
      echo "$FAILURES check(s) FAILED"
      exit 1
    fi
    echo "all web-server parity checks passed"
    ;;
  bench)
    echo "==> wrk -t$THREADS -c$CONNS -d$DURATION --latency (per server x path)"
    echo
    echo "| server | path | req/s | p50 | p75 | p90 | p99 |"
    echo "|---|---|---|---|---|---|---|"
    BENCH_PATHS=("/" "/$ASSET")
    if [[ -n "$WASM_ASSET" ]]; then BENCH_PATHS+=("/$WASM_ASSET"); fi
    for path in "${BENCH_PATHS[@]}"; do
      for s in "${SERVERS[@]}"; do
        IFS='|' read -r rps p50 p75 p90 p99 <<<"$(bench_one "$s" "$path")"
        echo "| $s | $path | $rps | $p50 | $p75 | $p90 | $p99 |"
      done
    done
    ;;
  *)
    echo "usage: $0 [test|bench] [dist] [duration] [conns] [threads]" >&2
    exit 2
    ;;
esac
