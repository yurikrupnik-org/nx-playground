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

[group('local-env')]
kompose:
    kubectl create ns dbs
    kompose convert --file ~/private/nx-playground/manifests/dockers/compose.yaml --namespace dbs --stdout | kubectl apply -f -

# Start full local dev environment (Kind + DBs + Secrets + Tilt)
[group('local-env')]
local-up *args:
    nu scripts/nu/mod.nu up {{args}}

# Tear down local dev environment
[group('local-env')]
local-down *args:
    nu scripts/nu/mod.nu down {{args}}

# Quick restart (keep cluster, redeploy apps)
[group('local-env')]
local-restart:
    nu scripts/nu/mod.nu down --keep-cluster
    tilt up

# Show environment status
[group('local-env')]
local-status:
    nu scripts/nu/mod.nu status
