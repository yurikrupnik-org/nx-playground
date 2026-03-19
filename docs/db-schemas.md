# Database Management

## Architecture

    schema.sql (source of truth)
         │
         ├──► db-fresh (local reset via psql)
         │
         ├──► migrate-add → .up.sql / .down.sql (you write the SQL)
         │         │
         │         ├──► sqlx migrate run (local + k8s dev via port-forward)
         │         │
         │         └──► Atlas Operator (k8s prod, consumes same .sql files via ConfigMaps)
         │
         └──► migrate-validate (diff schema.sql vs migration result)

- Local dev (docker-compose or k8s): sqlx migrate
- K8s dev (Tilt): sqlx migrate via port-forward, auto-triggered on file changes
- K8s prod: Atlas Operator via ConfigMaps (same .sql migration files)

All commands auto-detect whether postgres is running via docker-compose or k8s.

## Command Reference

### Local Development

| Command          | When                              |
|------------------|-----------------------------------|
| just db-fresh    | Schema changed, reset DB          |
| just db-seed     | Just re-seed data                 |
| just reset-db    | Docker is broken, nuke everything |
| just db-inspect  | See what's in the live DB         |

### Migrations (sqlx-cli, plain SQL)

| Command                          | When                                             |
|----------------------------------|--------------------------------------------------|
| just migrate-add add_feature     | Create .up.sql + .down.sql migration pair        |
| just migrate                     | Apply pending migrations locally                 |
| just migrate-revert              | Revert last migration                            |
| just migrate-test                | Test full migration path (drop → migrate → seed) |
| just migrate-status              | Check what's applied                             |
| just migrate-baseline            | Mark migrations as applied on existing DB        |
| just migrate-validate            | Verify schema.sql matches migration result       |

### K8s Cluster (Atlas Operator)

| Command                       | When                                        |
|-------------------------------|---------------------------------------------|
| just gen-schema-configmap     | Update AtlasSchema ConfigMap for Tilt/dev   |
| just gen-migrations-configmap | Update AtlasMigration ConfigMap for prod     |
| just cnpg-deploy dev          | Deploy CNPG + migrations (dev/staging/prod) |
| just cnpg-status              | Check cluster health                         |

## Scenarios

### I added a column / changed the schema

1. Edit `schema.sql` — add the column where it belongs
2. `just migrate-add add_column_to_table`
3. Edit `.up.sql`: `ALTER TABLE x ADD COLUMN y TEXT;`
4. Edit `.down.sql`: `ALTER TABLE x DROP COLUMN y;`
5. `just migrate` — applies to whichever postgres is running
6. `just migrate-validate` — confirm schema.sql and migrations match

Note: put new columns at the end of the CREATE TABLE in schema.sql,
since ALTER TABLE ADD COLUMN appends to the end in postgres.

### I dropped a column / removed something

1. Remove from `schema.sql`
2. `just migrate-add drop_column_from_table`
3. `.up.sql`: `ALTER TABLE x DROP COLUMN y;`
4. `.down.sql`: `ALTER TABLE x ADD COLUMN y TEXT;`
5. `just migrate`

### First time setup / fresh DB with no migration tracking

    just db-fresh
    just migrate-baseline

`db-fresh` creates the DB from schema.sql. `migrate-baseline` marks all existing
migrations as applied so future `migrate` calls only run new ones.

### I want to start clean (nuke everything)

Docker-compose:

    just reset-db

K8s:

    just db-fresh
    just migrate-baseline

### Test that migrations work end-to-end (prod path)

    just migrate-test

Drops the DB, recreates it, runs all migrations from scratch, then seeds.
Validates that the migration files can build the full schema from zero.

### Check if schema.sql drifted from migrations

    just migrate-validate

Creates two temp DBs: one from migrations, one from schema.sql.
Diffs the resulting schemas. Fails if they diverge.

### Revert a bad migration

    just migrate-revert

Runs the `.down.sql` of the last applied migration. Repeat to revert further.

### I deleted migration files and want to reset

1. Remove the migration files
2. `just db-fresh` — recreate DB from schema.sql
3. `just migrate-baseline` — re-baseline remaining migrations

### Deploy to K8s prod (CNPG)

    just cnpg-deploy prod

This generates the migrations ConfigMap and applies the CNPG kustomization.

### Add a new enum value

1. Edit `schema.sql` — add the value to the CREATE TYPE
2. `just migrate-add add_status_value`
3. `.up.sql`: `ALTER TYPE my_enum ADD VALUE 'new_value';`
4. `.down.sql`: enum value removal is not supported in postgres,
   so either leave empty with a comment or recreate the type

### Add a new table

1. Add full CREATE TABLE + indexes + triggers to `schema.sql`
2. `just migrate-add create_new_table`
3. Copy the CREATE TABLE block into `.up.sql`
4. `.down.sql`: `DROP TABLE IF EXISTS new_table;`
5. `just migrate`

### Rename a column

1. Update `schema.sql`
2. `just migrate-add rename_column_in_table`
3. `.up.sql`: `ALTER TABLE x RENAME COLUMN old TO new;`
4. `.down.sql`: `ALTER TABLE x RENAME COLUMN new TO old;`
5. `just migrate`
