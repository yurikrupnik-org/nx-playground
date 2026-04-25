# =============================================================================
# Database Management
# =============================================================================
# Source of truth: manifests/db/schema.sql
# Seed data:      manifests/db/seed.sql
# Migrations:     manifests/db/migrations/ (plain SQL, applied via sqlx)
# K8s prod:       Atlas Operator (consumes same migration SQL files via ConfigMaps)
#
# Workflow:
#   1. Edit schema.sql (source of truth)
#   2. just migrate-add <name>  → creates .up.sql + .down.sql
#   3. Write ALTER/CREATE in .up.sql, reversal in .down.sql
#   4. just migrate             → apply to local/cluster DB
#   5. just migrate-validate    → verify schema.sql matches migration result
#
# Works with both docker-compose and k8s (port-forwarded) postgres.
# Auto-detects which is running via _pg_cmd helper.

db_dir := "manifests/db"
db_url_local := "postgres://myuser:mypassword@localhost:5432/mydatabase"
db_url_cluster := "postgres://myuser:mypassword@localhost:5433/mydatabase"

# Helper: run psql via docker-compose or kubectl (whichever is available)
# Usage in bash scripts: $($PG_CMD) -d mydatabase -c "SELECT 1"
#         with stdin:    $($PG_CMD) -d mydatabase < file.sql
[private]
_pg_cmd := "if docker exec dockers-postgres-1 true 2>/dev/null; then echo 'docker exec -i dockers-postgres-1 psql -U myuser'; else echo 'kubectl exec -i -n dbs deployment/postgres -- psql -U myuser'; fi"

# =============================================================================
# Local Development
# =============================================================================

# Reset DB from schema.sql + seed.sql
db-fresh:
    #!/usr/bin/env bash
    set -euo pipefail
    PG=$({{ _pg_cmd }})
    echo "Terminating existing connections..."
    $PG -d postgres -c \
      "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'mydatabase' AND pid <> pg_backend_pid();" > /dev/null 2>&1 || true
    echo "Resetting mydatabase..."
    $PG -d postgres -c "DROP DATABASE IF EXISTS mydatabase;"
    $PG -d postgres -c "CREATE DATABASE mydatabase;"
    echo "Applying schema..."
    $PG -d mydatabase < {{ db_dir }}/schema.sql
    echo "Applying seed data..."
    $PG -d mydatabase < {{ db_dir }}/seed.sql
    echo "Done!"

# Apply seed data only
db-seed:
    #!/usr/bin/env bash
    PG=$({{ _pg_cmd }})
    $PG -d mydatabase < {{ db_dir }}/seed.sql

# Nuke docker + volumes + rebuild everything from scratch (docker-compose only)
reset-db:
    just docker-down
    docker volume prune -af
    docker compose -f manifests/dockers/compose.yaml up -d
    @echo "Waiting for PostgreSQL..."
    @until docker exec dockers-postgres-1 pg_isready -U myuser -q 2>/dev/null; do sleep 1; done
    just db-fresh

# Inspect live database schema (SQL output)
db-inspect:
    #!/usr/bin/env bash
    PG=$({{ _pg_cmd }})
    # pg_dump needs to run on the postgres host
    ${PG/psql/pg_dump} -d mydatabase --schema-only --no-owner --no-privileges

# =============================================================================
# Migrations (sqlx)
# =============================================================================
# sqlx tracks applied migrations in _sqlx_migrations table

# Create a new reversible migration (up + down files)
migrate-add name:
    #!/usr/bin/env bash
    set -euo pipefail
    timestamp=$(date -u +%Y%m%d%H%M%S)
    up="{{ db_dir }}/migrations/${timestamp}_{{ name }}.up.sql"
    down="{{ db_dir }}/migrations/${timestamp}_{{ name }}.down.sql"
    echo "-- Migration: {{ name }}" > "$up"
    echo "-- Revert: {{ name }}" > "$down"
    echo "Created:"
    echo "  $up"
    echo "  $down"
    echo "Edit both files, then run: just migrate"

# Apply pending migrations (local)
migrate:
    sqlx migrate run --source {{ db_dir }}/migrations --database-url "{{ db_url_local }}?sslmode=disable"

# Revert last migration (local)
migrate-revert:
    sqlx migrate revert --source {{ db_dir }}/migrations --database-url "{{ db_url_local }}?sslmode=disable"

# Apply migrations (kind cluster via port-forward on 5433)
migrate-cluster:
    sqlx migrate run --source {{ db_dir }}/migrations --database-url "{{ db_url_cluster }}?sslmode=disable"

# Check migration status
migrate-status:
    sqlx migrate info --source {{ db_dir }}/migrations --database-url "{{ db_url_local }}?sslmode=disable"

# Mark existing migrations as applied without running them
# Use when DB was created via db-fresh and you want to start using migrations
migrate-baseline:
    #!/usr/bin/env bash
    set -euo pipefail
    PG=$({{ _pg_cmd }})
    echo "Baselining migrations..."
    $PG -d mydatabase -q -c "
      CREATE TABLE IF NOT EXISTS _sqlx_migrations (
        version BIGINT PRIMARY KEY,
        description TEXT NOT NULL,
        installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
        success BOOLEAN NOT NULL DEFAULT true,
        checksum BYTEA NOT NULL,
        execution_time BIGINT NOT NULL DEFAULT 0
      );
    "
    for f in {{ db_dir }}/migrations/*.sql; do
      [ -f "$f" ] || continue
      [[ "$(basename "$f")" == *.down.sql ]] && continue
      basename=$(basename "$f")
      version=$(echo "$basename" | grep -oE '^[0-9]+')
      description=$(echo "$basename" | sed -E 's/^[0-9]+_//' | sed -E 's/\.(up\.)?sql$//' | tr '_' ' ')
      checksum=$(cat "$f" | shasum -a 384 | awk '{print $1}')
      $PG -d mydatabase -q -c "
        INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
        VALUES ($version, '$description', true, '\x$checksum', 0)
        ON CONFLICT (version) DO UPDATE SET checksum = '\x$checksum';
      "
      echo "  Baselined: $basename"
    done
    echo "Done! Run 'just migrate-status' to verify."

# Test migration path: drop DB → create → migrate → seed (validates prod path)
migrate-test:
    #!/usr/bin/env bash
    set -euo pipefail
    PG=$({{ _pg_cmd }})
    echo "Testing migration path..."
    $PG -d postgres -c \
      "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'mydatabase' AND pid <> pg_backend_pid();" > /dev/null 2>&1 || true
    $PG -d postgres -c "DROP DATABASE IF EXISTS mydatabase;"
    $PG -d postgres -c "CREATE DATABASE mydatabase;"
    sqlx migrate run --source {{ db_dir }}/migrations --database-url "{{ db_url_local }}?sslmode=disable"
    $PG -d mydatabase < {{ db_dir }}/seed.sql
    echo "Migration test passed!"

# Validate that schema.sql matches the result of running all migrations
migrate-validate:
    #!/usr/bin/env bash
    set -euo pipefail
    PG=$({{ _pg_cmd }})
    PG_DUMP=${PG/psql/pg_dump}
    echo "Validating schema.sql matches migrations..."
    # Create temp DB, apply migrations
    $PG -d postgres -c "DROP DATABASE IF EXISTS migrate_validate;" > /dev/null 2>&1
    $PG -d postgres -c "CREATE DATABASE migrate_validate;"
    sqlx migrate run --source {{ db_dir }}/migrations \
      --database-url "postgres://myuser:mypassword@localhost:5432/migrate_validate?sslmode=disable"
    # Create another temp DB, apply schema.sql
    $PG -d postgres -c "DROP DATABASE IF EXISTS schema_validate;" > /dev/null 2>&1
    $PG -d postgres -c "CREATE DATABASE schema_validate;"
    $PG -d schema_validate < {{ db_dir }}/schema.sql
    # Dump both and diff (exclude _sqlx_migrations table and comments)
    migrate_dump=$($PG_DUMP -d migrate_validate --schema-only --no-owner --no-privileges --no-comments \
      --exclude-table='_sqlx_migrations' | grep -v '^--' | grep -v '^$' | grep -vE '^\\(restrict|unrestrict)')
    schema_dump=$($PG_DUMP -d schema_validate --schema-only --no-owner --no-privileges --no-comments \
      | grep -v '^--' | grep -v '^$' | grep -vE '^\\(restrict|unrestrict)')
    # Cleanup
    $PG -d postgres -c "DROP DATABASE migrate_validate;" > /dev/null 2>&1
    $PG -d postgres -c "DROP DATABASE schema_validate;" > /dev/null 2>&1
    # Compare
    if diff <(echo "$migrate_dump") <(echo "$schema_dump") > /dev/null 2>&1; then
      echo "PASS: schema.sql and migrations produce identical schemas"
    else
      echo "FAIL: schema.sql and migrations diverge!"
      echo ""
      diff <(echo "$migrate_dump") <(echo "$schema_dump") || true
      exit 1
    fi

# =============================================================================
# K8s / CNPG (cluster) — Atlas Operator (free, no login required)
# =============================================================================

# Generate schema ConfigMap for AtlasSchema operator (Tilt/dev)
gen-schema-configmap:
    pg-cli configmap schema \
      --schema {{ db_dir }}/schema.sql \
      -o manifests/k8s/overlays/dev/schema-configmap.yaml

# Generate migrations ConfigMap for AtlasMigration operator (prod/staging)
# Only includes .up.sql and plain .sql files (excludes .down.sql)
gen-migrations-configmap:
    pg-cli configmap migrations \
      --migrations-dir {{ db_dir }}/migrations \
      -o manifests/k8s/base/cnpg/migrations-configmap.yaml \
      -o manifests/cnpg/base/migrations-configmap.yaml

# Deploy CNPG + migrations to cluster
cnpg-deploy env="dev":
    just gen-migrations-configmap
    kubectl apply -k manifests/cnpg/overlays/{{ env }}

# Check CNPG cluster status
cnpg-status ns="dbs":
    kubectl get clusters -n {{ ns }}
    kubectl get pods -n {{ ns }} -l cnpg.io/cluster
    @echo ""
    kubectl get atlasmigration -n {{ ns }} -o wide 2>/dev/null || echo "No AtlasMigration resources found"

# View CNPG logs
cnpg-logs ns="dbs":
    kubectl logs -n {{ ns }} -l cnpg.io/cluster --tail=100 -f
