---
name: db-migration
description: Add or modify a database schema migration (Atlas versioned mode). Use when changing table schemas, adding columns/indexes, or when the user mentions migrations, schema.sql, or AtlasMigration.
---

# Database migration workflow

Databases live under `manifests/db/<db>/` (zerg, todo, terran, tasks). Two modes:
**versioned** (has `migrations/` dir, AtlasMigration in cluster) and
**declarative** (schema.sql only, AtlasSchema). The `migrate-*` recipes fail on
declarative DBs by design. Full reference: `manifests/db/db.just` header.

## Versioned migration procedure (order matters)

1. `just migrate-add <db> <name>` — creates timestamped `.up.sql` in
   `manifests/db/<db>/migrations/`
2. Write the ALTER/CREATE statements in the created file
3. **Also update `manifests/db/<db>/schema.sql`** to the equivalent end state —
   schema.sql must always equal the result of replaying all migrations
4. `just migrate <db>` — apply locally (docker compose DB must be up: `just docker-up`)
5. `just migrate-validate <db>` — verifies schema.sql == migrations result; fix
   drift before continuing
6. `just gen-migrations-configmap <db>` — re-hashes atlas.sum and regenerates the
   cluster ConfigMap. Never skip: a stale atlas.sum breaks the Atlas Operator.

## Rules

- Never edit an already-applied migration file — add a new one
  (`just migrate-baseline <db>` exists for marking applied without running)
- After editing/deleting ANY file in `migrations/`: `just migrate-hash <db>`
  (gen-migrations-configmap does this automatically)
- Local rollback during dev: `just db-fresh <db>` (rebuild from schema.sql + seed),
  then `just migrate-baseline <db>`
- Test the full path with `just migrate-test <db>` (drop → create → migrate → seed)
- SeaORM entities in `libs/` are hand-maintained — update them to match the new
  schema, then `cargo nextest run -p <affected domain crate>` (tests use
  testcontainers; docker required)
