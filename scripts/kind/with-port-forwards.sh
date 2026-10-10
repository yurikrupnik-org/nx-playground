#!/usr/bin/env bash
# Run a command while kubectl port-forwards are up, and always tear them down.
#
#   bash scripts/kind/with-port-forwards.sh <ns>/<svc>:<local>:<remote>... -- <command> [args...]
#
# Why a script and not inline Taskfile shell: go-task's interpreter (mvdan/sh)
# gives `$!` a job id, not a PID, and has no `kill` builtin, so a backgrounded
# `kubectl port-forward` there can never be stopped. It outlives the task and
# keeps the task's output pipe open (`task … | tail` then hangs forever).
# Here bash owns real PIDs and the EXIT trap kills them.
#
# Uses the current kube context (like scripts/tasks/db.yml); set KUBECONFIG or
# the context before calling. Written for bash 3.2 (macOS /bin/bash) as well.
set -euo pipefail

forwards=()
while [ $# -gt 0 ] && [ "$1" != "--" ]; do
  forwards+=("$1")
  shift
done
if [ "${1:-}" != "--" ] || [ $# -lt 2 ] || [ ${#forwards[@]} -eq 0 ]; then
  echo "usage: $0 <ns>/<svc>:<local>:<remote>... -- <command> [args...]" >&2
  exit 2
fi
shift

pids=()
cleanup() {
  # `${a[@]+…}`: expanding an empty array under `set -u` is an error in bash 3.2.
  for pid in ${pids[@]+"${pids[@]}"}; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

for spec in "${forwards[@]}"; do
  target=${spec%%:*}   # ns/svc
  ports=${spec#*:}     # local:remote
  ns=${target%%/*}
  svc=${target#*/}
  local_port=${ports%%:*}
  kubectl port-forward -n "$ns" "svc/$svc" "$ports" >/dev/null 2>&1 &
  pid=$!
  pids+=("$pid")
  ready=false
  for _ in $(seq 60); do
    if nc -z localhost "$local_port" 2>/dev/null; then
      ready=true
      break
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "port-forward $spec exited (is svc/$svc in namespace $ns?)" >&2
      exit 1
    fi
    sleep 0.5
  done
  if [ "$ready" != true ]; then
    echo "port-forward $spec did not listen on localhost:$local_port within 30s" >&2
    exit 1
  fi
done

# Not `exec`: the EXIT trap must still run to stop the forwards.
"$@"
