# Local environment lifecycle (docker compose, kind, secrets) — imported by the root justfile.

# Start local env dbs (Postgres, Redis, NATS) via Docker Compose
[group('local-env')]
docker-up:
    devkit dev up -d

# Remove local env db
[group('local-env')]
docker-down:
    devkit dev down

# Generate + apply the shared Secret into the local kind cluster from vals refs.
# `vals eval` resolves each ref+gcpsecrets:// at apply time (no plaintext on disk
# or in git). A RANDOM name is stamped so it won't clobber the kustomize-managed
# `zerg-shared-secrets` during migration — rename once deployments point at it.
#   just kind-secret                   # random name, namespace zerg
#   just kind-secret my-secrets        # explicit name (POSITIONAL, not name=...)
#   just kind-secret my-secrets apps   # explicit name + namespace
[group('local-env')]
kind-secret name=`printf "zerg-secrets-%s" "$(openssl rand -hex 4)"` namespace="zerg":
    @echo "{{ name }}" | grep -Eq '^[a-z0-9]([-a-z0-9]*[a-z0-9])?$' || { echo "✗ invalid Secret name '{{ name }}' — pass it POSITIONALLY (RFC1123: lowercase alphanumeric/-). Use: just kind-secret <name> [namespace]"; exit 1; }
    kubectl create namespace {{ namespace }} --dry-run=client -o yaml | kubectl apply -f -
    vals eval -f manifests/secrets/secret.vals.yaml \
      | sed "s/SECRET_NAME_PLACEHOLDER/{{ name }}/" \
      | kubectl apply -n {{ namespace }} -f -
    @echo "✓ applied Secret '{{ name }}' to namespace '{{ namespace }}' (random name — update refs, then rename)"

# Start full local dev environment (Kind + DBs + Secrets + Tilt)
[group('local-env')]
local-up *args:
    devkit up {{ args }}

# Tear down local dev environment
[group('local-env')]
local-down *args:
    devkit down {{ args }}

# Quick restart (keep cluster, redeploy apps)
[group('local-env')]
local-restart:
    devkit down --keep-cluster
    tilt up

# Show environment status
[group('local-env')]
local-status:
    devkit status

# Credentials for the `x` CLI/TUI (apps/x/cli). Guarded routes reject an
# anonymous request with 401; x sends either a bearer token ($X_TOKEN) or the
# session cookie its OpenAPI document declares ($X_SESSION).

# Mint a Keycloak access token for the local `terran` realm (direct-access grant,
# 5-minute lifetime) and print it. Needs `docker compose up -d keycloak`:
#   export X_TOKEN=$(just x-token)
#   cargo run -p x_cli -- get assets
[group('local-env')]
x-token user="test@terran.dev" password="test" realm="terran" client="terran-api" secret="local-dev-secret":
    @curl -fsS -X POST "http://localhost:8088/realms/{{ realm }}/protocol/openid-connect/token" \
      -H 'content-type: application/x-www-form-urlencoded' \
      -d grant_type=password -d "client_id={{ client }}" -d "client_secret={{ secret }}" \
      --data-urlencode "username={{ user }}" --data-urlencode "password={{ password }}" \
      | jq -er .access_token

# Log in to a running API through its own password route and print the session
# cookie value. The cookie name is the one the document declares — read it with
# `cargo run -p x_cli -- api show <api> | jq .components.securitySchemes`:
#   export X_SESSION=$(just x-session http://127.0.0.1:8080 zerg_session me@example.com hunter2)
[group('local-env')]
x-session origin cookie email password:
    @curl -fsS -D - -o /dev/null -X POST "{{ origin }}/api/auth/login/password" \
      -H 'content-type: application/json' \
      --data-binary "$(jq -nc --arg e '{{ email }}' --arg p '{{ password }}' '{email:$e,password:$p}')" \
      | tr -d '\r' \
      | sed -n 's/^[Ss]et-[Cc]ookie: *{{ cookie }}=\([^;]*\).*/\1/p' \
      | head -n 1
