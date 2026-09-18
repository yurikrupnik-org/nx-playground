#!/usr/bin/env bash
# End-to-end smoke test for the todo vertical on the cluster.
# Path exercised: port-forward -> todo-web (nginx) -> /api proxy -> todo-api
# -> Postgres, with todo-worker consuming the NATS event.
# Invoked by `just todo-smoke`.
set -euo pipefail

NS=todo
PORT="${TODO_SMOKE_PORT:-5240}"
BASE="http://127.0.0.1:$PORT"

kubectl -n "$NS" port-forward svc/todo-web "$PORT:8080" >/dev/null 2>&1 &
PF_PID=$!
trap 'kill "$PF_PID" 2>/dev/null || true' EXIT

for i in $(seq 1 50); do
  curl -fsS -o /dev/null "$BASE/health" 2>/dev/null && break
  [[ $i == 50 ]] && { echo "FATAL: port-forward to todo-web never came up" >&2; exit 1; }
  sleep 0.2
done
echo "ok   todo-web /health"

curl -fsS "$BASE/" | grep -q '<div id="root">' || { echo "FAIL: / is not the SPA" >&2; exit 1; }
echo "ok   / serves the SPA"

TITLE="smoke-$(date +%s)"
TODO_JSON=$(curl -fsS -X POST -H 'Content-Type: application/json' \
  -d "{\"title\":\"$TITLE\",\"priority\":\"high\"}" "$BASE/api/todos")
ID=$(jq -re '.id' <<<"$TODO_JSON")
echo "ok   POST /api/todos created $ID"

curl -fsS "$BASE/api/todos?limit=100000" | jq -e --arg id "$ID" 'map(.id) | index($id)' >/dev/null \
  || { echo "FAIL: created todo not in list" >&2; exit 1; }
echo "ok   GET /api/todos lists it"

curl -fsS -X POST "$BASE/api/todos/$ID/complete" | jq -e '.completed == true' >/dev/null \
  || { echo "FAIL: complete did not flip the flag" >&2; exit 1; }
echo "ok   POST /api/todos/$ID/complete"

curl -fsS -X DELETE "$BASE/api/todos/$ID"
echo "ok   DELETE /api/todos/$ID"

# Worker consumed the 3 lifecycle events (created/completed/deleted): assert
# the todo-worker durable consumer's ack floor advanced past the stream tail
# via the NATS monitoring endpoint (log-grepping is unreliable at DEBUG level).
NATS_PORT="${TODO_SMOKE_NATS_PORT:-5241}"
kubectl -n "$NS" port-forward svc/nats "$NATS_PORT:8222" >/dev/null 2>&1 &
NATS_PF_PID=$!
trap 'kill "$PF_PID" "$NATS_PF_PID" 2>/dev/null || true' EXIT

consumer_state() { # -> "<ack_floor_seq> <num_pending>"
  curl -fsS "http://127.0.0.1:$NATS_PORT/jsz?accounts=true&consumers=true" 2>/dev/null |
    jq -r '[.. | objects | select(.name? == "todo-worker" and has("ack_floor"))
            | "\(.ack_floor.consumer_seq // 0) \(.num_pending // 0)"][0] // empty'
}

ok=""
for _ in $(seq 1 25); do
  state=$(consumer_state || true)
  if [[ -n "$state" ]]; then
    read -r acked pending <<<"$state"
    if [[ "$acked" -ge 3 && "$pending" -eq 0 ]]; then ok=1; break; fi
  fi
  sleep 0.4
done
if [[ -n "$ok" ]]; then
  echo "ok   todo-worker consumed all events (acked=$acked, pending=$pending)"
else
  echo "FAIL: todo-worker consumer did not drain the TODOS stream (state: ${state:-none})" >&2
  exit 1
fi

echo
echo "todo cluster smoke test passed"
