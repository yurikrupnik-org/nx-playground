-- tasks — application database role (local dev; a template for other environments).
--
-- Only `apps/zerg/tasks` connects here. `zerg_api` is deliberately given no role and
-- no grants on this database: after `docs/adr-tasks-service-boundary.md` Phase 3 the
-- gRPC service is the *only* path to these rows, so a future in-process shortcut is
-- impossible rather than merely discouraged.
--
-- Applied by `just db-fresh tasks` after schema.sql.

DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'tasks_app') THEN
    CREATE ROLE tasks_app LOGIN PASSWORD 'tasks_app'
      NOSUPERUSER NOCREATEDB NOCREATEROLE;
  END IF;
END$$;

GRANT USAGE ON SCHEMA public TO tasks_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO tasks_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO tasks_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA public
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO tasks_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
  GRANT USAGE, SELECT ON SEQUENCES TO tasks_app;

-- Belt and braces: nothing may reach these tables via the implicit PUBLIC role.
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
