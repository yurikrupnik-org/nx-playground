# =============================================================================
# Database Management (multi-DB)
# =============================================================================
# Layout: manifests/db/<db>/{schema.sql, seed.sql, migrations/, k8s/}
# Source of truth: schema.sql per DB
# Migrations: plain SQL applied via sqlx (local) and Atlas Operator (cluster)
#
# Workflow:
#   1. Edit manifests/db/<db>/schema.sql (source of truth)
#   2. just migrate-add <db> <name>   → creates .up.sql + .down.sql
#   3. Write ALTER/CREATE in .up.sql, reversal in .down.sql
#   4. just migrate <db>               → apply locally
#   5. just migrate-validate <db>      → verify schema.sql == migrations result
#   6. just gen-migrations-configmap <db>  → regenerate cluster ConfigMap
#
# Local: docker-compose postgres on :5432 with super-user `myuser`.
# Cluster: kind/CNPG via port-forward on :5433.

# =============================================================================
# Helpers
# =============================================================================

# Run psql via docker-compose or kubectl (whichever is available)
[private]
_pg_cmd := "if docker exec dockers-postgres-1 true 2>/dev/null; then echo 'docker exec -i dockers-postgres-1 psql -U myuser'; else echo 'kubectl exec -i -n dbs deployment/postgres -- psql -U myuser'; fi"

# Per-DB connection URL. Add new (db, env) pairs as you onboard databases.
# env is one of: local, cluster
[private]
_db-url db env="local":
    #!/usr/bin/env bash
    case "{{ db }}-{{ env }}" in
      zerg-local)    echo "postgres://myuser:mypassword@localhost:5432/zerg?sslmode=disable" ;;
      zerg-cluster)  echo "postgres://zerg:zerg@localhost:5433/zerg?sslmode=disable" ;;
      *)
        echo "unknown db/env: {{ db }}/{{ env }} — add it to _db-url in manifests/db/db.just" >&2
        exit 1
        ;;
    esac

# =============================================================================
# Discovery
# =============================================================================

# List all configured databases (folders under manifests/db/)
db-list:
    @ls -d manifests/db/*/ 2>/dev/null | xargs -n1 basename

# =============================================================================
# Local Development
# =============================================================================

# Reset <db> from schema.sql + seed.sql (local docker-compose)
db-fresh db:
    #!/usr/bin/env bash
    set -euo pipefail
    PG=$({{ _pg_cmd }})
    echo "Terminating existing connections to {{ db }}..."
    $PG -d postgres -c \
      "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{{ db }}' AND pid <> pg_backend_pid();" > /dev/null 2>&1 || true
    echo "Resetting {{ db }}..."
    $PG -d postgres -c "DROP DATABASE IF EXISTS {{ db }};"
    $PG -d postgres -c "CREATE DATABASE {{ db }};"
    echo "Applying schema..."
    $PG -d {{ db }} < manifests/db/{{ db }}/schema.sql
    if [ -f manifests/db/{{ db }}/seed.sql ]; then
      echo "Applying seed data..."
      $PG -d {{ db }} < manifests/db/{{ db }}/seed.sql
    fi
    echo "Done!"

# Apply seed data only for <db>
db-seed db:
    #!/usr/bin/env bash
    PG=$({{ _pg_cmd }})
    $PG -d {{ db }} < manifests/db/{{ db }}/seed.sql

# Inspect live <db> schema (SQL output)
db-inspect db:
    #!/usr/bin/env bash
    PG=$({{ _pg_cmd }})
    ${PG/psql/pg_dump} -d {{ db }} --schema-only --no-owner --no-privileges

# Nuke docker + volumes + rebuild from scratch (docker-compose only); reapplies all DBs
reset-db:
    #!/usr/bin/env bash
    set -euo pipefail
    just docker-down
    docker volume prune -af
    docker compose -f manifests/dockers/compose.yaml up -d
    echo "Waiting for PostgreSQL..."
    until docker exec dockers-postgres-1 pg_isready -U myuser -q 2>/dev/null; do sleep 1; done
    for d in manifests/db/*/; do
      just db-fresh "$(basename "$d")"
    done

# =============================================================================
# Migrations (sqlx) — per DB
# =============================================================================
# sqlx tracks applied migrations in _sqlx_migrations table per database.

# Create a new forward-only migration for <db>
# (Single .up.sql per migration — Atlas Operator can't disambiguate up/down pairs.
#  For local rollback during dev, use `just db-fresh <db>` to reset from schema.sql.)
migrate-add db name:
    #!/usr/bin/env bash
    set -euo pipefail
    timestamp=$(date -u +%Y%m%d%H%M%S)
    mkdir -p manifests/db/{{ db }}/migrations
    f="manifests/db/{{ db }}/migrations/${timestamp}_{{ name }}.up.sql"
    echo "-- Migration: {{ name }}" > "$f"
    echo "Created: $f"
    echo "Edit, then run: just migrate {{ db }}"

# Apply pending migrations for <db> (local)
migrate db:
    sqlx migrate run \
      --source manifests/db/{{ db }}/migrations \
      --database-url "$(just _db-url {{ db }} local)"

# Apply migrations for <db> against kind cluster (port-forwarded :5433)
migrate-cluster db:
    sqlx migrate run \
      --source manifests/db/{{ db }}/migrations \
      --database-url "$(just _db-url {{ db }} cluster)"

# Show migration status for <db>
migrate-status db:
    sqlx migrate info \
      --source manifests/db/{{ db }}/migrations \
      --database-url "$(just _db-url {{ db }} local)"

# Mark existing migrations as applied without running them (use after db-fresh)
migrate-baseline db:
    #!/usr/bin/env bash
    set -euo pipefail
    PG=$({{ _pg_cmd }})
    echo "Baselining {{ db }} migrations..."
    $PG -d {{ db }} -q -c "
      CREATE TABLE IF NOT EXISTS _sqlx_migrations (
        version BIGINT PRIMARY KEY,
        description TEXT NOT NULL,
        installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
        success BOOLEAN NOT NULL DEFAULT true,
        checksum BYTEA NOT NULL,
        execution_time BIGINT NOT NULL DEFAULT 0
      );
    "
    for f in manifests/db/{{ db }}/migrations/*.sql; do
      [ -f "$f" ] || continue
      [[ "$(basename "$f")" == *.down.sql ]] && continue
      basename=$(basename "$f")
      version=$(echo "$basename" | grep -oE '^[0-9]+')
      description=$(echo "$basename" | sed -E 's/^[0-9]+_//' | sed -E 's/\.(up\.)?sql$//' | tr '_' ' ')
      checksum=$(cat "$f" | shasum -a 384 | awk '{print $1}')
      $PG -d {{ db }} -q -c "
        INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
        VALUES ($version, '$description', true, '\x$checksum', 0)
        ON CONFLICT (version) DO UPDATE SET checksum = '\x$checksum';
      "
      echo "  Baselined: $basename"
    done
    echo "Done! Run 'just migrate-status {{ db }}' to verify."

# Test migration path for <db>: drop → create → migrate → seed
migrate-test db:
    #!/usr/bin/env bash
    set -euo pipefail
    PG=$({{ _pg_cmd }})
    echo "Testing {{ db }} migration path..."
    $PG -d postgres -c \
      "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '{{ db }}' AND pid <> pg_backend_pid();" > /dev/null 2>&1 || true
    $PG -d postgres -c "DROP DATABASE IF EXISTS {{ db }};"
    $PG -d postgres -c "CREATE DATABASE {{ db }};"
    sqlx migrate run \
      --source manifests/db/{{ db }}/migrations \
      --database-url "$(just _db-url {{ db }} local)"
    if [ -f manifests/db/{{ db }}/seed.sql ]; then
      $PG -d {{ db }} < manifests/db/{{ db }}/seed.sql
    fi
    echo "{{ db }} migration test passed!"

# Verify schema.sql matches the result of running all migrations for <db>
migrate-validate db:
    #!/usr/bin/env bash
    set -euo pipefail
    PG=$({{ _pg_cmd }})
    PG_DUMP=${PG/psql/pg_dump}
    mig_db="{{ db }}_migrate_validate"
    sch_db="{{ db }}_schema_validate"
    echo "Validating {{ db }}/schema.sql matches migrations..."
    $PG -d postgres -c "DROP DATABASE IF EXISTS $mig_db;" > /dev/null 2>&1
    $PG -d postgres -c "CREATE DATABASE $mig_db;"
    sqlx migrate run \
      --source manifests/db/{{ db }}/migrations \
      --database-url "postgres://myuser:mypassword@localhost:5432/$mig_db?sslmode=disable"
    $PG -d postgres -c "DROP DATABASE IF EXISTS $sch_db;" > /dev/null 2>&1
    $PG -d postgres -c "CREATE DATABASE $sch_db;"
    $PG -d $sch_db < manifests/db/{{ db }}/schema.sql
    migrate_dump=$($PG_DUMP -d $mig_db --schema-only --no-owner --no-privileges --no-comments \
      --exclude-table='_sqlx_migrations' | grep -v '^--' | grep -v '^$' | grep -vE '^\\(restrict|unrestrict)')
    schema_dump=$($PG_DUMP -d $sch_db --schema-only --no-owner --no-privileges --no-comments \
      | grep -v '^--' | grep -v '^$' | grep -vE '^\\(restrict|unrestrict)')
    $PG -d postgres -c "DROP DATABASE $mig_db;" > /dev/null 2>&1
    $PG -d postgres -c "DROP DATABASE $sch_db;" > /dev/null 2>&1
    if diff <(echo "$migrate_dump") <(echo "$schema_dump") > /dev/null 2>&1; then
      echo "PASS: {{ db }} schema.sql and migrations produce identical schemas"
    else
      echo "FAIL: {{ db }} schema.sql and migrations diverge!"
      echo ""
      diff <(echo "$migrate_dump") <(echo "$schema_dump") || true
      exit 1
    fi

# =============================================================================
# K8s ConfigMap generation (Atlas Operator) — per DB
# =============================================================================

# Generate schema ConfigMap for <db> (used by AtlasSchema declarative mode)
gen-schema-configmap db:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p manifests/db/{{ db }}/k8s/base
    pg-cli configmap schema \
      --schema manifests/db/{{ db }}/schema.sql \
      --name {{ db }}-schema \
      -o manifests/db/{{ db }}/k8s/base/schema-configmap.yaml

# Recompute the Atlas checksum manifest (atlas.sum) for <db>'s migrations dir.
# Run after any edit/add/delete of migration files; required by Atlas Operator.
migrate-hash db:
    atlas migrate hash --dir "file://manifests/db/{{ db }}/migrations"

# Generate migrations ConfigMap for <db> (used by AtlasMigration versioned mode)
# Always re-hashes first so atlas.sum is consistent with the SQL files.
gen-migrations-configmap db: (migrate-hash db)
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p manifests/db/{{ db }}/k8s/base
    pg-cli configmap migrations \
      --migrations-dir manifests/db/{{ db }}/migrations \
      --name {{ db }}-migrations \
      -o manifests/db/{{ db }}/k8s/base/migrations-configmap.yaml

# Regenerate ConfigMaps for every DB under manifests/db/
gen-all-configmaps:
    #!/usr/bin/env bash
    set -euo pipefail
    for d in manifests/db/*/; do
      [[ -d "$d/migrations" ]] || continue
      db=$(basename "$d")
      just gen-migrations-configmap "$db"
    done

# =============================================================================
# Cluster status (CNPG)
# =============================================================================

# Show CNPG cluster status (cluster-wide, not per-DB)
cnpg-status ns="dbs":
    kubectl get clusters -n {{ ns }}
    kubectl get pods -n {{ ns }} -l cnpg.io/cluster
    @echo ""
    kubectl get atlasmigration -A -o wide 2>/dev/null || echo "No AtlasMigration resources found"

# Tail CNPG logs
cnpg-logs ns="dbs":
    kubectl logs -n {{ ns }} -l cnpg.io/cluster --tail=100 -f
