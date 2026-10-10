#!/usr/bin/env bash
# Throwaway Postgres for the e2e suite, run in the FOREGROUND so Playwright's
# `webServer` treats it like any server: the port opens when it is up.
#
# Lifetime is the subtle part. Playwright tears a webServer down by SIGKILLing
# its process group, which kills the attached `docker run` client but leaves
# the container running — and it checks the port BEFORE launching, so a
# leftover would fail the next run before this script could remove it. The
# watchdog below therefore lives in its OWN process group (`set -m`), survives
# the group kill, and removes the container the moment the Playwright runner
# (our parent) is gone. A stale container is replaced at start regardless.
#
# Env (set by playwright.config.ts): TODO_E2E_PG_CONTAINER, TODO_E2E_PG_PORT.
set -euo pipefail

name="${TODO_E2E_PG_CONTAINER:?}"
port="${TODO_E2E_PG_PORT:?}"
runner=$PPID

docker rm -f "$name" >/dev/null 2>&1 || true

set -m
(
  while kill -0 "$runner" 2>/dev/null; do sleep 1; done
  docker rm -f "$name" >/dev/null 2>&1 || true
) >/dev/null 2>&1 </dev/null &
set +m

exec docker run --rm --name "$name" \
  -e POSTGRES_USER=todo -e POSTGRES_PASSWORD=todo -e POSTGRES_DB=todo \
  -p "127.0.0.1:${port}:5432" \
  postgres:17-alpine \
  -c log_min_messages=warning
