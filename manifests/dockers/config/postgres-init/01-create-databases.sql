-- Local dev only: create the extra databases used by terran + supporting services.
-- Runs automatically on a fresh (ephemeral) Postgres data dir via
-- /docker-entrypoint-initdb.d. The zerg database is created by POSTGRES_DB.
-- Keycloak uses an embedded H2 store in `start-dev`, so it needs no database here.
CREATE DATABASE terran;
CREATE DATABASE flagsmith;
CREATE DATABASE directus;
