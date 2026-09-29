#!/usr/bin/env bash
# Installs the taskgraph CLI from the rolling `taskgraph-cli-latest` release
# (.github/workflows/taskgraph-cli.yml) and puts a `task` shim AHEAD of go-task
# on PATH, so every later `task X` in the job runs as `taskgraph shim X`:
# RunStarted.origin records the CI run/job/sha, and every event is appended to
# $TASKGRAPH_EVENTS_OUT, which the job uploads as `taskgraph-events-<job>-<attempt>`
# for the in-cluster insights sync to pull (docs/ci-insights.md).
#
# Telemetry must never fail CI: every failure below is a ::warning:: and exit 0,
# and the shim is only written once the binary is verified, so a failed download
# leaves plain go-task exactly where setup-task put it.
#
# Inputs (environment): RUNNER_TEMP, GITHUB_PATH, GITHUB_ENV, GITHUB_JOB,
# GITHUB_REPOSITORY, GITHUB_EVENT_NAME, GH_TOKEN (gh), PR_HEAD_SHA (pull_request only).
set -uo pipefail

readonly TAG=taskgraph-cli-latest
readonly ASSET=taskgraph-x86_64-unknown-linux-musl.tar.gz

warn() {
  echo "::warning title=taskgraph telemetry::$* — task calls in this job run untraced"
  exit 0
}

# Resolved BEFORE the shim exists: this is the binary the shim hands off to.
real_task=$(command -v task) || warn "no go-task on PATH; run go-task/setup-task before this action"

# The release carries one static musl build; anything else keeps plain go-task.
[ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || warn "no taskgraph build for $(uname -s)-$(uname -m)"

dir="$RUNNER_TEMP/taskgraph"
bin="$dir/bin"
download=$(mktemp -d) || warn "mktemp failed"
trap 'rm -rf "$download"' EXIT

gh release download "$TAG" --repo "$GITHUB_REPOSITORY" --dir "$download" \
  --pattern "$ASSET" --pattern "$ASSET.sha256" ||
  warn "could not download $ASSET from release $TAG"
(cd "$download" && sha256sum --check --strict "$ASSET.sha256") ||
  warn "$ASSET does not match its .sha256"
{ mkdir -p "$bin" && tar -xzf "$download/$ASSET" -C "$bin" taskgraph; } ||
  warn "could not unpack $ASSET"
# `help shim` also proves the subcommand exists: a release predating it would
# turn every `task` call into a usage error instead of an observed run.
if ! { "$bin/taskgraph" --version && "$bin/taskgraph" help shim >/dev/null; }; then
  rm -f "$bin/taskgraph"
  warn "the downloaded taskgraph does not run or has no \`shim\` subcommand"
fi

# Environment first, PATH last: the shim must never be reachable without
# TASKGRAPH_TASK_BIN, or `task` would resolve back to itself.
{
  echo "TASKGRAPH_TASK_BIN=$real_task"
  echo "TASKGRAPH_EVENTS_OUT=$dir/events-$GITHUB_JOB.jsonl"
  # CI has no NATS: events go to the file above, and the sync publishes them.
  echo "TASKGRAPH_OFFLINE=1"
  # On pull_request GITHUB_SHA is the synthetic merge commit; the commit the
  # insights attribute is the PR head.
  if [ "$GITHUB_EVENT_NAME" = pull_request ] && [ -n "${PR_HEAD_SHA:-}" ]; then
    echo "TASKGRAPH_CI_SHA=$PR_HEAD_SHA"
  fi
} >>"$GITHUB_ENV" || warn "could not write GITHUB_ENV"

# The default repeats TASKGRAPH_TASK_BIN so a step that clears it cannot loop.
cat >"$bin/task" <<EOF || warn "could not write the task shim"
#!/bin/sh
export TASKGRAPH_TASK_BIN="\${TASKGRAPH_TASK_BIN:-$real_task}"
exec "$bin/taskgraph" shim "\$@"
EOF
chmod +x "$bin/task" || warn "could not make the task shim executable"
echo "$bin" >>"$GITHUB_PATH" || warn "could not write GITHUB_PATH"
echo "task -> taskgraph shim -> $real_task"
