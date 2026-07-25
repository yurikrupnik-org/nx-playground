-- terran — application database role (local dev; a template for other environments).
--
-- The API MUST connect as this NON-superuser, NOBYPASSRLS role so the Row-Level
-- Security policies in schema.sql actually take effect: superusers and table owners
-- otherwise bypass RLS, which silently disables the tenant-isolation safety net.
--
-- Applied by `just db-fresh terran` after schema.sql. In the CNPG cluster the app
-- connects as the managed `terran` role (a non-superuser DB owner; FORCE RLS applies).

DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'terran_app') THEN
    CREATE ROLE terran_app LOGIN PASSWORD 'terran_app'
      NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
  END IF;
END$$;

GRANT USAGE ON SCHEMA public, util TO terran_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO terran_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO terran_app;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA util TO terran_app;

-- Tables/sequences added by later migrations (run as the owner) inherit these grants.
ALTER DEFAULT PRIVILEGES IN SCHEMA public
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO terran_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
  GRANT USAGE, SELECT ON SEQUENCES TO terran_app;
