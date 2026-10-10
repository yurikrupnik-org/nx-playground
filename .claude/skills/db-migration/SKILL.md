---
name: db-migration
description: Add or modify a database schema migration (Atlas versioned mode). Use when changing table schemas, adding columns/indexes, or when the user mentions migrations, schema.sql, or AtlasMigration.
---

# Database migration workflow

Databases live under `manifests/db/<db>/` (zerg, todo, terran, tasks). Two modes:
**versioned** (has `migrations/` dir, AtlasMigration in cluster) and
**declarative** (schema.sql only, AtlasSchema). The `migrate-*` tasks fail on
declarative DBs by design. Full reference: `scripts/tasks/db.yml` header.

## Versioned migration procedure (order matters)

1. `task migrate-add DB=<db> NAME=<name>` — creates timestamped `.up.sql` in
   `manifests/db/<db>/migrations/`
2. Write the ALTER/CREATE statements in the created file
3. **Also update `manifests/db/<db>/schema.sql`** to the equivalent end state —
   schema.sql must always equal the result of replaying all migrations
4. `task migrate DB=<db>` — apply locally (docker compose DB must be up: `task docker-up`)
5. `task migrate-validate DB=<db>` — verifies schema.sql == migrations result; fix
   drift before continuing
6. `task gen-migrations-configmap DB=<db>` — re-hashes atlas.sum and regenerates the
   cluster ConfigMap. Never skip: a stale atlas.sum breaks the Atlas Operator.

## Rules

- Never edit an already-applied migration file — add a new one
  (`task migrate-baseline DB=<db>` exists for marking applied without running)
- After editing/deleting ANY file in `migrations/`: `task migrate-hash DB=<db>`
  (gen-migrations-configmap does this automatically)
- Local rollback during dev: `task db-fresh DB=<db>` (rebuild from schema.sql + seed),
  then `task migrate-baseline DB=<db>`
- Test the full path with `task migrate-test DB=<db>` (drop → create → migrate → seed)
- SeaORM entities in `libs/` are hand-maintained — update them to match the new
  schema, then `cargo nextest run -p <affected domain crate>` (tests use
  testcontainers; docker required)
