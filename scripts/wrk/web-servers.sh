#!/usr/bin/env bash
# Compare the static web server images (nginx / caddy / static-web-server)
# serving the SAME SPA dist, built from manifests/dockers/Dockerfile targets.
#
#   web-servers.sh test  [dist]                          behavior-parity checks
#   web-servers.sh bench [dist] [duration] [conns] [threads]   wrk + p50/75/90/99 table
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

cleanup() {
  local s
  for s in "${SERVERS[@]}"; do docker rm -f "web-cmp-$s" >/dev/null 2>&1 || true; done
}
trap cleanup EXIT

# --- dist -------------------------------------------------------------------
if [[ ! -f "$DIST/index.html" ]]; then
  echo "==> $DIST/index.html missing; building todo-web"
  bun nx run todo-web:build
fi
[[ -f "$DIST/index.html" ]] || { echo "FATAL: no index.html in $DIST" >&2; exit 1; }

ASSET="$(cd "$DIST" && find assets -name '*.js' 2>/dev/null | head -n1 || true)"
[[ -n "$ASSET" ]] || { echo "FATAL: no js asset under $DIST/assets" >&2; exit 1; }

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
  local s=$1 base code body headers
  base="http://127.0.0.1:$(port_of "$s")"
  echo "== $s =="

  code=$(curl -s -o /dev/null -w '%{http_code}' "$base/health")
  expect "$s" "/health -> 200 (got $code)" "$([[ $code == 200 ]]; echo $?)"

  body=$(curl -fsS "$base/")
  expect "$s" "/ serves index.html" "$(grep -q '<div id="root">' <<<"$body"; echo $?)"

  body=$(curl -fsS "$base/some/client/route")
  expect "$s" "SPA fallback serves index.html" "$(grep -q '<div id="root">' <<<"$body"; echo $?)"

  headers=$(curl -fsS -D - -o /dev/null "$base/$ASSET")
  expect "$s" "asset $ASSET -> 200 + js content-type" \
    "$(grep -qi 'content-type:.*javascript' <<<"$headers"; echo $?)"
  expect "$s" "asset has long-lived cache-control" \
    "$(grep -qi 'cache-control:.*\(immutable\|max-age=[0-9]\{4,\}\)' <<<"$headers"; echo $?)"

  headers=$(curl -fsS -H 'Accept-Encoding: gzip' -D - -o /dev/null "$base/$ASSET")
  expect "$s" "asset served gzip-compressed" \
    "$(grep -qi 'content-encoding: gzip' <<<"$headers"; echo $?)"
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
    for path in "/" "/$ASSET"; do
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
