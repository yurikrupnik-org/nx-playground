#!/bin/bash
# Create the extra databases that other compose services expect on the shared
# Postgres instance (flagsmith, directus). The official postgres image only
# creates the single POSTGRES_DB, so anything else must be provisioned here.
#
# Scripts in /docker-entrypoint-initdb.d run once, when the data directory is
# first initialised. This compose service has no persistent volume, so a
# `docker compose down` (or `up --force-recreate postgres`) re-runs it.
set -euo pipefail

for db in flagsmith directus; do
	if psql -tAc "SELECT 1 FROM pg_database WHERE datname = '${db}'" \
		--username "${POSTGRES_USER}" --dbname postgres | grep -q 1; then
		echo "database already exists: ${db}"
	else
		psql -v ON_ERROR_STOP=1 --username "${POSTGRES_USER}" --dbname postgres \
			-c "CREATE DATABASE ${db};"
		echo "created database: ${db}"
	fi
done
