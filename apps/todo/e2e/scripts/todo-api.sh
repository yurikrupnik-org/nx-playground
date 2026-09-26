#!/usr/bin/env bash
# Apply the todo migrations to the e2e Postgres, then exec todo-api in place.
#
# Playwright starts webServers in array order and waits for each before the
# next, but the Postgres entry is "ready" when its port opens — before the
# schema exists. Migrating here, on the api's own readiness path, keeps the
# ordering honest: /healthz only answers once the trigger is installed.
#
# psql runs inside the container, so the host needs docker and cargo only
# (no sqlx-cli). Files are applied in name order; `atlas.sum` is skipped.
set -euo pipefail

name="${TODO_E2E_PG_CONTAINER:?}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
migrations="$root/manifests/db/todo/migrations"

# The image's entrypoint runs a socket-only bootstrap server (initdb, CREATE
# DATABASE) before the real one: probe over TCP and query the todo database,
# or `pg_isready` answers before the database exists.
ready() {
  docker exec "$name" psql -h 127.0.0.1 -U todo -d todo -qAt -c 'select 1' >/dev/null 2>&1
}
for _ in $(seq 1 150); do
  if ready; then break; fi
  sleep 0.2
done
ready

for file in "$migrations"/*.sql; do
  echo "migrate: $(basename "$file")"
  docker exec -i "$name" psql -q -v ON_ERROR_STOP=1 -U todo -d todo <"$file"
done

cd "$root"
exec cargo run -q -p todo_api
