#!/usr/bin/env bash
# Test script for db-worker
#
# Usage:
#   ./test.sh              # Run all tests with mock Dapr sidecar
#   ./test.sh direct       # Direct HTTP to worker (starts mock Dapr sidecar)
#   ./test.sh dapr         # Via real Dapr sidecar (requires Dapr running)
#   ./test.sh redis        # Via redis-cli publish (requires Redis running)

set -euo pipefail

WORKER_PORT="${APP_PORT:-9091}"
WORKER_HOST="${WORKER_HOST:-localhost}"
DAPR_PORT="${DAPR_HTTP_PORT:-3500}"
MOCK_DAPR_PORT="${MOCK_DAPR_PORT:-3500}"
REDIS_HOST="${REDIS_HOST:-localhost}"
REDIS_PORT="${REDIS_PORT:-6379}"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

CLEANUP_PIDS=()

cleanup() {
  for pid in "${CLEANUP_PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

pass() { echo -e "${GREEN}PASS${NC} $1"; }
fail() { echo -e "${RED}FAIL${NC} $1"; exit 1; }
info() { echo -e "${YELLOW}----${NC} $1"; }

# ── Mock Dapr sidecar ─────────────────────────────────────────

start_mock_dapr() {
  info "Starting mock Dapr sidecar on port $MOCK_DAPR_PORT"

  python3 -c "
import http.server, json, threading

state = {}

class DaprHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass

    def do_GET(self):
        # GET /v1.0/state/{store}/{key}
        parts = self.path.split('/')
        if len(parts) >= 5 and parts[1] == 'v1.0' and parts[2] == 'state':
            key = '/'.join(parts[4:])
            store = parts[3]
            full_key = f'{store}:{key}'
            if full_key in state:
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(json.dumps(state[full_key]).encode())
            else:
                self.send_response(204)
                self.end_headers()
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        content_len = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_len) if content_len else b'[]'
        parts = self.path.split('?')[0].split('/')

        # POST /v1.0/state/{store} - save state
        if len(parts) == 4 and parts[1] == 'v1.0' and parts[2] == 'state':
            store = parts[3]
            items = json.loads(body)
            for item in items:
                full_key = f'{store}:{item[\"key\"]}'
                state[full_key] = item['value']
            self.send_response(204)
            self.end_headers()
            return

        # POST /v1.0-alpha1/state/{store}/query - query state
        if 'query' in self.path:
            store = parts[3]
            results = [{'key': k.split(':', 1)[1], 'data': v}
                       for k, v in state.items() if k.startswith(f'{store}:')]
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps({'results': results}).encode())
            return

        # POST /v1.0/publish/{pubsub}/{topic} - publish (accept and discard)
        if len(parts) >= 5 and parts[2] == 'publish':
            self.send_response(204)
            self.end_headers()
            return

        self.send_response(404)
        self.end_headers()

    def do_DELETE(self):
        parts = self.path.split('/')
        if len(parts) >= 5 and parts[1] == 'v1.0' and parts[2] == 'state':
            key = '/'.join(parts[4:])
            store = parts[3]
            full_key = f'{store}:{key}'
            state.pop(full_key, None)
            self.send_response(204)
            self.end_headers()
            return
        self.send_response(404)
        self.end_headers()

server = http.server.HTTPServer(('127.0.0.1', $MOCK_DAPR_PORT), DaprHandler)
server.serve_forever()
" &
  CLEANUP_PIDS+=($!)

  # Wait for mock to be ready
  for i in $(seq 1 10); do
    if curl -s -o /dev/null "http://localhost:${MOCK_DAPR_PORT}/" 2>/dev/null; then
      pass "Mock Dapr sidecar ready"
      return
    fi
    sleep 0.2
  done
  fail "Mock Dapr sidecar failed to start"
}

# ── Health checks ──────────────────────────────────────────────

test_health() {
  info "Testing health endpoints"

  status=$(curl -s -o /dev/null -w '%{http_code}' "http://${WORKER_HOST}:${WORKER_PORT}/health")
  [ "$status" = "200" ] && pass "/health → 200" || fail "/health → $status"

  status=$(curl -s -o /dev/null -w '%{http_code}' "http://${WORKER_HOST}:${WORKER_PORT}/ready")
  [ "$status" = "200" ] && pass "/ready → 200" || fail "/ready → $status"
}

# ── Dapr subscription discovery ───────────────────────────────

test_subscription_discovery() {
  info "Testing Dapr subscription discovery"

  body=$(curl -s "http://${WORKER_HOST}:${WORKER_PORT}/dapr/subscribe")
  echo "$body" | python3 -m json.tool > /dev/null 2>&1 \
    && pass "/dapr/subscribe returns valid JSON" \
    || fail "/dapr/subscribe returned invalid JSON: $body"

  echo "$body" | python3 -c "
import sys, json
subs = json.load(sys.stdin)
assert isinstance(subs, list), 'Expected array'
assert len(subs) > 0, 'Expected at least 1 subscription'
print(f'  Subscriptions: {len(subs)}')
for s in subs:
    print(f'    topic={s[\"topic\"]}  route={s[\"route\"]}  pubsub={s[\"pubsubname\"]}')
" && pass "Subscription discovery valid" || fail "Subscription structure invalid"
}

# ── Helper: send CloudEvent to worker ─────────────────────────

send_event() {
  local op="$1" entity="${2:-tasks}" payload="$3" expected_status="${4:-SUCCESS}"
  local event_id="test-${op}-$(date +%s%N)"

  resp=$(curl -s -w '\n%{http_code}' -X POST \
    "http://${WORKER_HOST}:${WORKER_PORT}/events/db-tasks-pg" \
    -H "Content-Type: application/json" \
    -d "{
      \"data\": {
        \"id\": \"${event_id}\",
        \"operation\": \"${op}\",
        \"entity_type\": \"${entity}\",
        \"payload\": ${payload},
        \"retry_count\": 0
      },
      \"specversion\": \"1.0\",
      \"type\": \"com.dapr.event.sent\",
      \"source\": \"test-script\",
      \"id\": \"evt-${event_id}\",
      \"topic\": \"db.tasks.pg\",
      \"pubsubname\": \"pubsub-nats\"
    }")

  http_code=$(echo "$resp" | tail -1)
  body=$(echo "$resp" | sed '$d')

  [ "$http_code" = "200" ] || fail "${op} HTTP → $http_code: $body"

  actual_status=$(echo "$body" | python3 -c "import sys,json; print(json.load(sys.stdin)['status'])" 2>/dev/null || echo "PARSE_ERROR")
  [ "$actual_status" = "$expected_status" ] \
    && pass "${op} → ${actual_status}" \
    || fail "${op}: expected ${expected_status}, got ${actual_status} ($body)"
}

# ── CRUD tests ────────────────────────────────────────────────

test_crud() {
  info "Testing CRUD operations"
  send_event "create" "tasks" '{"title": "Test task", "status": "pending"}'
  send_event "read"   "tasks" '{"id": "some-id"}'
  send_event "update" "tasks" '{"id": "some-id", "title": "Updated", "status": "done"}'
  send_event "delete" "tasks" '{"id": "some-id"}'
  send_event "query"  "tasks" '{"filter": {"status": "pending"}}'
}

test_unsupported_op() {
  info "Testing unsupported operations (should DROP)"
  send_event "vector_search" "tasks" '{}' "DROP"
  send_event "graph_traverse" "tasks" '{}' "DROP"
  send_event "time_series_write" "tasks" '{}' "DROP"
}

# ── Via Dapr sidecar HTTP API ─────────────────────────────────

test_dapr_publish() {
  info "Publishing via Dapr HTTP API (port $DAPR_PORT)"

  for op in create read update delete; do
    status=$(curl -s -o /dev/null -w '%{http_code}' -X POST \
      "http://${WORKER_HOST}:${DAPR_PORT}/v1.0/publish/pubsub-nats/db.tasks.pg" \
      -H "Content-Type: application/json" \
      -d "{
        \"id\": \"dapr-${op}-1\",
        \"operation\": \"${op}\",
        \"entity_type\": \"tasks\",
        \"payload\": {\"title\": \"Dapr ${op} test\"},
        \"retry_count\": 0
      }")
    [ "$status" = "200" ] || [ "$status" = "204" ] \
      && pass "Dapr publish ${op} → $status" \
      || fail "Dapr publish ${op} → $status"
  done
}

# ── Via redis-cli ─────────────────────────────────────────────

test_redis_publish() {
  info "Publishing via redis-cli"

  for op in create read update delete; do
    redis-cli -h "$REDIS_HOST" -p "$REDIS_PORT" \
      PUBLISH db.tasks.pg "{\"id\":\"redis-${op}-1\",\"operation\":\"${op}\",\"entity_type\":\"tasks\",\"payload\":{\"title\":\"Redis ${op} test\"},\"retry_count\":0}" \
      > /dev/null 2>&1 \
      && pass "Redis publish ${op}" \
      || fail "Redis publish ${op}"
  done
}

# ── Main ──────────────────────────────────────────────────────

MODE="${1:-direct}"

echo ""
echo "========================================="
echo "  db-worker test suite (mode: $MODE)"
echo "========================================="
echo ""

case "$MODE" in
  direct)
    start_mock_dapr
    test_health
    test_subscription_discovery
    test_crud
    test_unsupported_op
    ;;
  dapr)
    test_health
    test_subscription_discovery
    test_dapr_publish
    ;;
  redis)
    test_redis_publish
    ;;
  all)
    start_mock_dapr
    test_health
    test_subscription_discovery
    test_crud
    test_unsupported_op
    test_dapr_publish 2>/dev/null || info "Dapr sidecar not available, skipping"
    test_redis_publish 2>/dev/null || info "Redis not available, skipping"
    ;;
  *)
    echo "Usage: $0 [direct|dapr|redis|all]"
    exit 1
    ;;
esac

echo ""
echo -e "${GREEN}All tests passed!${NC}"
