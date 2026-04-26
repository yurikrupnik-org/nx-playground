#!/usr/bin/env just --justfile
# import 'manifests/db/db.just'  # re-enable after removing duplicate db_url_* defs from this file

default:
    just -l

dam:
  just cluster-create yurikrupnik-local
  just cluster-oidc-upload yurikrupnik-local
  # just cluster-grant yurikrupnik-local # check if needed
  export GITHUB_TOKEN=$(gh auth token)
  just cluster-bootstrap yurikrupnik-local
#  just cluster-bootstrap yurikrupnik-local
#  kubectl config use-context kind-kind-yurik
# =============================================================================
# Cluster lifecycle: kind (local) or gke (real). Provider read from inputs.yaml.
# Usage: `just cluster-up local-yk` or `just cluster-up paidevo-cluster`
# =============================================================================

gcp_user := `gcloud config get-value account 2>/dev/null | sed 's/@.*//'`
github_account := env("GITHUB_ACCOUNT", "yurikrupnik")
github_repo := env("GITHUB_REPO", "nx-playground")

# Render all KCL outputs for a cluster (cluster-config, wif, kind-only artifacts)
cluster-render CLUSTER:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="clusters/dev/{{ CLUSTER }}"
    test -f "$dir/inputs.yaml" || { echo "missing $dir/inputs.yaml"; exit 1; }
    mkdir -p "$dir/wif" "$dir/cluster" "$dir/oidc"
    settings=$(mktemp)
    yq '{"kcl_options": (to_entries | map({"key": .key, "value": .value}))}' \
        "$dir/inputs.yaml" > "$settings"
    provider=$(yq -r '.clusterProvider' "$dir/inputs.yaml")
    kcl run kcl/main.k -Y "$settings" -D target=cluster-config \
        > "$dir/cluster-config.yaml"
    kcl run kcl/main.k -Y "$settings" -D target=wif \
        > "$dir/wif/wif.yaml"
    if [ "$provider" = "kind" ]; then
        kcl run kcl/main.k -Y "$settings" -D target=kind-config \
            > "$dir/cluster/kind-config.yaml"
        kcl run kcl/main.k -Y "$settings" -D target=oidc-config \
            | yq -o=json > "$dir/oidc/openid-configuration"
    fi
    rm -f "$settings"
    echo "Rendered $dir (provider=$provider)"

# Create or attach to the cluster (kind: create; gke: get-credentials)
cluster-create CLUSTER: (cluster-render CLUSTER)
    #!/usr/bin/env bash
    set -euo pipefail
    dir="clusters/dev/{{ CLUSTER }}"
    provider=$(yq -r '.clusterProvider' "$dir/inputs.yaml")
    name=$(yq -r '.clusterName' "$dir/inputs.yaml")
    if [ "$provider" = "kind" ]; then
        kind create cluster --name "$name" --config "$dir/cluster/kind-config.yaml"
    else
        location=$(yq -r '.gkeLocation' "$dir/inputs.yaml")
        project=$(yq -r '.gcpProject' "$dir/inputs.yaml")
        gcloud container clusters get-credentials "$name" \
            --location "$location" --project "$project"
    fi

# Publish OIDC discovery to GCS (kind only — GKE uses its own metadata server)
cluster-oidc-upload CLUSTER:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="clusters/dev/{{ CLUSTER }}"
    provider=$(yq -r '.clusterProvider' "$dir/inputs.yaml")
    if [ "$provider" != "kind" ]; then
        echo "skip: OIDC upload only applies to kind clusters"; exit 0
    fi
    oidc_id=$(yq -r '.oidcId // .clusterName' "$dir/inputs.yaml")
    bucket=$(yq -r '.oidcBucket' "$dir/inputs.yaml")
    kubectl get --raw /openid/v1/jwks > /tmp/keys.json
    gcloud storage cp /tmp/keys.json "gs://$bucket/$oidc_id/keys.json"
    gcloud storage cp "$dir/oidc/openid-configuration" \
        "gs://$bucket/$oidc_id/.well-known/openid-configuration"
    echo "OIDC published to gs://$bucket/$oidc_id/"

# Bootstrap Flux at the per-cluster path. Reads githubBranch / githubAccount / githubRepo
# from inputs.yaml so kind/gke clusters can track different branches per cluster.
cluster-bootstrap CLUSTER:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="clusters/dev/{{ CLUSTER }}"
    branch=$(yq -r '.githubBranch // "main"' "$dir/inputs.yaml")
    owner=$(yq -r ".githubAccount // \"{{ github_account }}\"" "$dir/inputs.yaml")
    repo=$(yq -r ".githubRepo // \"{{ github_repo }}\"" "$dir/inputs.yaml")
    auth=$(yq -r '.githubAuth // "pat"' "$dir/inputs.yaml")

    # flux bootstrap pushes a commit to the remote BEFORE applying in-cluster.
    # If a previous attempt pushed but failed in-cluster, local is now behind.
    # Sync with remote before re-running so the next push is fast-forward.
    current_branch=$(git rev-parse --abbrev-ref HEAD)
    if [ "$current_branch" = "$branch" ]; then
        echo "==> syncing local '$branch' with origin (rebase)"
        git fetch origin "$branch"
        if ! git pull --rebase origin "$branch"; then
            echo "✗ rebase failed — resolve conflicts manually then re-run"
            exit 1
        fi
    else
        echo "warning: local branch is '$current_branch' but cluster wants '$branch' — skipping pre-pull"
    fi

    echo "==> flux bootstrap github owner=$owner repo=$repo branch=$branch path=$dir auth=$auth"
    case "$auth" in
      app)
        # GitHub App auth is not supported by `flux bootstrap` — do it manually:
        #   1. flux install         (deploys flux-system components)
        #   2. create Secret with App credentials
        #   3. create GitRepository with provider=github + secretRef
        #   4. create root Kustomization pointing at $dir
        app_id=$(yq -r '.githubAppId' "$dir/inputs.yaml")
        install_id=$(yq -r '.githubAppInstallationId' "$dir/inputs.yaml")
        pem_path=$(yq -r '.githubAppPemPath' "$dir/inputs.yaml" | sed "s|^~|$HOME|")
        test -f "$pem_path" || { echo "missing PEM at $pem_path"; exit 1; }

        echo "==> flux install"
        flux install --components-extra=image-reflector-controller,image-automation-controller >/dev/null
        kubectl -n flux-system wait --for=condition=Ready pod -l app=source-controller --timeout=2m

        echo "==> creating Secret flux-system/flux-system (GitHub App credentials)"
        kubectl -n flux-system create secret generic flux-system \
            --from-literal=githubAppID="$app_id" \
            --from-literal=githubAppInstallationID="$install_id" \
            --from-file=githubAppPrivateKey="$pem_path" \
            --dry-run=client -o yaml | kubectl apply -f -

        echo "==> creating GitRepository + root Kustomization"
        # Write YAML to tmp file (heredoc + just dedent leave column-4 indent — strip it)
        cat <<EOF | sed 's/^    //' > /tmp/flux-source.yaml
    apiVersion: source.toolkit.fluxcd.io/v1
    kind: GitRepository
    metadata:
      name: flux-system
      namespace: flux-system
    spec:
      interval: 1m
      ref:
        branch: $branch
      provider: github
      secretRef:
        name: flux-system
      url: https://github.com/$owner/$repo.git
    ---
    apiVersion: kustomize.toolkit.fluxcd.io/v1
    kind: Kustomization
    metadata:
      name: flux-system
      namespace: flux-system
    spec:
      interval: 10m
      path: ./$dir
      prune: true
      sourceRef:
        kind: GitRepository
        name: flux-system
    EOF
        kubectl apply -f /tmp/flux-source.yaml
        rm -f /tmp/flux-source.yaml

        echo "==> reconcile"
        flux reconcile source git flux-system
        flux reconcile kustomization flux-system
        ;;
      pat)
        personal=$(yq -r '.githubPersonal // true' "$dir/inputs.yaml")
        personal_flag=""
        if [ "$personal" = "true" ]; then personal_flag="--personal"; fi
        # --token-auth uses HTTPS+GITHUB_TOKEN instead of SSH deploy keys
        # (orgs commonly disable deploy keys; HTTPS auth always works)
        flux bootstrap github \
            --token-auth \
            --owner="$owner" --repository="$repo" --branch="$branch" --path="$dir" \
            $personal_flag
        ;;
      *) echo "unknown githubAuth: $auth (expected pat or app)"; exit 1 ;;
    esac

# Bind KSAs to GSAs (kind: WIF provider; gke: native Workload Identity)
# Idempotent — re-run safely.
cluster-grant CLUSTER:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="clusters/dev/{{ CLUSTER }}"
    provider=$(yq -r '.clusterProvider' "$dir/inputs.yaml")
    project=$(yq -r '.gcpProject' "$dir/inputs.yaml")
    project_number=$(gcloud projects describe "$project" --format='value(projectNumber)')
    yq -r '.workloads[] | [.namespace, .ksa, .clouds.gcp.gsaEmail, (.clouds.gcp.providerSuffix // .name)] | @tsv' \
        "$dir/inputs.yaml" | while IFS=$'\t' read -r ns ksa gsa suffix; do
        if [ "$provider" = "kind" ]; then
            member="principal://iam.googleapis.com/projects/$project_number/locations/global/workloadIdentityPools/local-clusters/subject/system:serviceaccount:$ns:$ksa"
        else
            member="serviceAccount:$project.svc.id.goog[$ns/$ksa]"
        fi
        echo "binding $ksa@$ns -> $gsa ($member)"
        gcloud iam service-accounts add-iam-policy-binding "$gsa" \
            --role=roles/iam.workloadIdentityUser \
            --member="$member" \
            --project="$project" >/dev/null
    done
    echo "WI bindings applied"

# Full lifecycle: render + create + (kind: oidc upload) + grant + flux bootstrap
cluster-up CLUSTER: (cluster-create CLUSTER) (cluster-oidc-upload CLUSTER) (cluster-grant CLUSTER) (cluster-bootstrap CLUSTER)
    @echo "Cluster {{ CLUSTER }} ready"

# Tear down (kind only — refuses for gke to avoid wrecking the real cluster)
cluster-down CLUSTER:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="clusters/dev/{{ CLUSTER }}"
    provider=$(yq -r '.clusterProvider' "$dir/inputs.yaml")
    name=$(yq -r '.clusterName' "$dir/inputs.yaml")
    if [ "$provider" = "kind" ]; then
        kind delete cluster --name "$name"
    else
        echo "refusing to delete real GKE cluster '$name' — do it manually"; exit 1
    fi

# One-time per-account: GCS bucket + WIF pool + per-workload providers (kind only).
# Idempotent: 409 conflicts are swallowed; re-run safely after editing workloads.
wif-bootstrap CLUSTER:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="clusters/dev/{{ CLUSTER }}"
    test -f "$dir/inputs.yaml" || { echo "missing $dir/inputs.yaml"; exit 1; }
    provider=$(yq -r '.clusterProvider' "$dir/inputs.yaml")
    if [ "$provider" != "kind" ]; then
        echo "skip: wif-bootstrap only applies to kind clusters (gke uses native Workload Identity)"
        exit 0
    fi
    project=$(yq -r '.gcpProject' "$dir/inputs.yaml")
    bucket=$(yq -r '.oidcBucket' "$dir/inputs.yaml")
    oidc_id=$(yq -r '.oidcId // .clusterName' "$dir/inputs.yaml")
    issuer="https://storage.googleapis.com/$bucket/$oidc_id"
    pool=local-clusters

    echo "==> ensuring bucket gs://$bucket (public read)"
    gcloud storage buckets create "gs://$bucket" \
        --project="$project" --location=US --uniform-bucket-level-access 2>/dev/null \
        || echo "    bucket exists"
    gcloud storage buckets add-iam-policy-binding "gs://$bucket" \
        --member=allUsers --role=roles/storage.objectViewer >/dev/null

    echo "==> ensuring WIF pool '$pool' in $project"
    gcloud iam workload-identity-pools create "$pool" \
        --location=global --project="$project" \
        --display-name="Local kind clusters" 2>/dev/null \
        || echo "    pool exists"

    yq -r '.workloads[] | [.namespace, .ksa, (.clouds.gcp.providerSuffix // .name)] | @tsv' \
        "$dir/inputs.yaml" | while IFS=$'\t' read -r ns ksa suffix; do
        provider_id="kind-$oidc_id-$suffix"
        echo "==> ensuring WIF provider '$provider_id' (sub=system:serviceaccount:$ns:$ksa)"
        gcloud iam workload-identity-pools providers create-oidc "$provider_id" \
            --workload-identity-pool="$pool" \
            --location=global \
            --project="$project" \
            --issuer-uri="$issuer" \
            --allowed-audiences="$issuer" \
            --attribute-mapping="google.subject=assertion.sub" \
            --attribute-condition="assertion.sub == 'system:serviceaccount:$ns:$ksa'" 2>/dev/null \
            || echo "    provider exists (delete + recreate to change subject mapping)"
    done

    echo
    echo "WIF bootstrap done. Next: just cluster-up {{ CLUSTER }}"

# Install mkcert root CA into a kind cluster's cert-manager namespace as Secret
# 'mkcert-root-ca' so the ca-local ClusterIssuer can sign certs. Idempotent.
# Run AFTER flux has reconciled cert-manager (the namespace must exist).
kind-ca-install CLUSTER:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="clusters/dev/{{ CLUSTER }}"
    provider=$(yq -r '.clusterProvider' "$dir/inputs.yaml")
    if [ "$provider" != "kind" ]; then
        echo "skip: kind-ca-install only applies to kind clusters"; exit 0
    fi
    command -v mkcert >/dev/null || { echo "mkcert not installed (brew install mkcert)"; exit 1; }
    mkcert -install >/dev/null 2>&1 || true
    caroot=$(mkcert -CAROOT)
    test -f "$caroot/rootCA.pem" || { echo "mkcert root CA missing at $caroot"; exit 1; }
    test -f "$caroot/rootCA-key.pem" || { echo "mkcert root CA key missing at $caroot"; exit 1; }

    name=$(yq -r '.clusterName' "$dir/inputs.yaml")
    ctx="kind-$name"
    echo "==> waiting for cert-manager namespace in $ctx (flux must reconcile first)"
    until kubectl --context="$ctx" get ns cert-manager >/dev/null 2>&1; do sleep 5; done

    echo "==> applying Secret cert-manager/mkcert-root-ca"
    kubectl --context="$ctx" -n cert-manager create secret tls mkcert-root-ca \
        --cert="$caroot/rootCA.pem" --key="$caroot/rootCA-key.pem" \
        --dry-run=client -o yaml | kubectl --context="$ctx" apply -f -
    echo "ca-local ClusterIssuer should report Ready shortly"

# Convenience aliases — pass the cluster directory name from clusters/dev/
# Default to yurikrupnik-local for kind, paidevo-gke for gke; override per call.
kind-up CLUSTER="yurikrupnik-local": (cluster-up CLUSTER)
kind-down CLUSTER="yurikrupnik-local": (cluster-down CLUSTER)
gke-up CLUSTER="paidevo-gke": (cluster-up CLUSTER)

# Full quality check for Rust monorepo (read-only, CI-safe)
check: fmt-check lint test audit
    @echo "All checks passed!"

# Check formatting without modifying files
fmt-check:
    cargo fmt --all --check

# Format all Rust code
fmt:
    cargo fmt --all

# Run clippy linter on all packages
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run all tests
test:
    cargo nextest run --workspace

# Security and dependency checks
audit:
    cargo audit --ignore RUSTSEC-2023-0071  # RSA timing vulnerability - no fix available
    cargo deny check --config .cargo/deny.toml

# Quick check (no tests, just compile and lint)
check-quick: fmt-check
    cargo check --workspace
    cargo clippy --workspace --all-targets -- -D warnings

# Show outdated dependencies
outdated:
    cargo outdated --workspace

# Update Cargo.lock to latest compatible versions
update:
    cargo update

# Upgrade Cargo.toml versions to latest (requires cargo-edit)
upgrade:
    cargo upgrade --workspace --incompatible
    cargo update

_docker-up:
    docker compose -f manifests/dockers/compose.yaml up -d

# Remove local env db
docker-down:
    docker compose -f manifests/dockers/compose.yaml down

run *args:
    bacon {{ args }}

# Run zerg web dev server
web:
    cd apps/zerg/web && bun run dev

# =============================================================================
# Atlas Migration Configuration
# =============================================================================

default_db := "mydatabase"
db_url_local := "postgres://myuser:mypassword@localhost:5432"
db_url_cluster := "postgres://myuser:mypassword@localhost:5433"
schemas_dir := "manifests/schemas"
migrations_dir := "manifests/migrations"

# =============================================================================
# Schema Development (HCL)
# =============================================================================

# Validate HCL schema syntax
schema-validate db=default_db:
    atlas schema inspect -u "file://{{schemas_dir}}/{{db}}.hcl" --format '{{{{sql .}}}}' > /dev/null
    @echo "Schema {{db}}.hcl is valid"

# Preview schema as SQL (without applying)
schema-sql db=default_db:
    atlas schema inspect -u "file://{{schemas_dir}}/{{db}}.hcl" --format '{{{{sql .}}}}'

# Preview schema diff (what would change)
schema-diff db=default_db:
    atlas schema diff \
      --from "{{db_url_local}}/{{db}}?sslmode=disable" \
      --to "file://{{schemas_dir}}/{{db}}.hcl" \
      --dev-url "docker://postgres/18/dev"

# Apply schema directly (dev only - use migrations for prod)
schema-apply db=default_db:
    atlas schema apply \
      --url "{{db_url_local}}/{{db}}?sslmode=disable" \
      --to "file://{{schemas_dir}}/{{db}}.hcl" \
      --dev-url "docker://postgres/18/dev"

# =============================================================================
# Migration Generation (HCL → SQL)
# =============================================================================

# Generate new migration from schema changes
migrate-diff db=default_db name="":
    atlas migrate diff {{name}} \
      --dir "file://{{migrations_dir}}/{{db}}" \
      --to "file://{{schemas_dir}}/{{db}}.hcl" \
      --dev-url "docker://postgres/18/dev"
    @echo "Migration generated in {{migrations_dir}}/{{db}}/"

# =============================================================================
# Migration Application
# =============================================================================

# Apply migrations (local)
migrate db=default_db:
    atlas migrate apply --dir "file://{{migrations_dir}}/{{db}}" --url "{{db_url_local}}/{{db}}?sslmode=disable"

# Apply migrations (cluster)
migrate-cluster db=default_db:
    atlas migrate apply --dir "file://{{migrations_dir}}/{{db}}" --url "{{db_url_cluster}}/{{db}}?sslmode=disable"

# Apply migrations (alias for migrate mydatabase)
migrate-all:
    @echo "Migrating mydatabase..."
    atlas migrate apply --dir "file://{{migrations_dir}}/mydatabase" --url "{{db_url_local}}/mydatabase?sslmode=disable"
    @echo "Migrations complete!"

# Apply migrations to cluster
migrate-all-cluster:
    atlas migrate apply --dir "file://{{migrations_dir}}/mydatabase" --url "{{db_url_cluster}}/mydatabase?sslmode=disable"

# Check migration status
migrate-status db=default_db:
    atlas migrate status --dir "file://{{migrations_dir}}/{{db}}" --url "{{db_url_local}}/{{db}}?sslmode=disable"

# Dry-run migrations
migrate-dry db=default_db:
    atlas migrate apply --dir "file://{{migrations_dir}}/{{db}}" --url "{{db_url_local}}/{{db}}?sslmode=disable" --dry-run

# Lint migrations
migrate-lint db=default_db:
    atlas migrate lint --dir "file://{{migrations_dir}}/{{db}}" --dev-url "docker://postgres/18/dev" --latest 1

# Hash migrations (after manual edits)
migrate-hash db=default_db:
    atlas migrate hash --dir "file://{{migrations_dir}}/{{db}}"

# =============================================================================
# Development Migration Commands (UNSAFE for production)
# =============================================================================

# DEV ONLY: Refresh migrations after editing SQL files
# Usage:
#   just dev-migrate-refresh          - Re-hash and apply migrations
#   just dev-migrate-refresh fresh    - Drop DB, re-hash, and apply (clean slate)
dev-migrate-refresh db=default_db *args:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "⚠️  DEV ONLY - Do not use in production!"
    echo ""

    if [[ "{{args}}" == *"fresh"* ]]; then
        echo "1. Dropping and recreating {{db}}..."
        psql "{{db_url_local}}/postgres?sslmode=disable" -c "DROP DATABASE IF EXISTS {{db}};"
        psql "{{db_url_local}}/postgres?sslmode=disable" -c "CREATE DATABASE {{db}};"
    else
        echo "1. Skipping database drop (use 'fresh' flag to drop and recreate)"
    fi

    echo "2. Re-hashing migrations..."
    atlas migrate hash --dir "file://{{migrations_dir}}/{{db}}"

    echo "3. Applying migrations..."
    atlas migrate apply --dir "file://{{migrations_dir}}/{{db}}" --url "{{db_url_local}}/{{db}}?sslmode=disable"

    echo ""
    echo "Done! Migrations refreshed."

# =============================================================================
# Production Migration Commands (SAFE)
# =============================================================================

# PROD: Safe migration workflow (lint → status → dry-run → apply)
prod-migrate db=default_db url=db_url_local:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Production Migration Workflow ==="
    echo ""

    echo "1. Linting migrations for destructive changes..."
    atlas migrate lint --dir "file://{{migrations_dir}}/{{db}}" --dev-url "docker://postgres/18/dev" --latest 1 || true
    echo ""

    echo "2. Current migration status..."
    atlas migrate status --dir "file://{{migrations_dir}}/{{db}}" --url "{{url}}/{{db}}?sslmode=disable"
    echo ""

    echo "3. Dry-run (what will be applied)..."
    atlas migrate apply --dir "file://{{migrations_dir}}/{{db}}" --url "{{url}}/{{db}}?sslmode=disable" --dry-run
    echo ""

    read -p "4. Apply these migrations? [y/N] " confirm
    if [[ "$confirm" =~ ^[Yy]$ ]]; then
        echo "Applying migrations..."
        atlas migrate apply --dir "file://{{migrations_dir}}/{{db}}" --url "{{url}}/{{db}}?sslmode=disable"
        echo ""
        echo "=== Migration Complete ==="
    else
        echo "Migration cancelled."
        exit 0
    fi

# PROD: Preview only (no changes)
prod-migrate-preview db=default_db url=db_url_local:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Migration Preview (No Changes) ==="
    echo ""

    echo "1. Linting..."
    atlas migrate lint --dir "file://{{migrations_dir}}/{{db}}" --dev-url "docker://postgres/18/dev" --latest 1 || true
    echo ""

    echo "2. Status..."
    atlas migrate status --dir "file://{{migrations_dir}}/{{db}}" --url "{{url}}/{{db}}?sslmode=disable"
    echo ""

    echo "3. Dry-run..."
    atlas migrate apply --dir "file://{{migrations_dir}}/{{db}}" --url "{{url}}/{{db}}?sslmode=disable" --dry-run
    echo ""

    echo "Run 'just prod-migrate {{db}}' to apply."

# =============================================================================
# Database Inspection & Docs
# =============================================================================

# Inspect live database (HCL output)
db-inspect db=default_db:
    atlas schema inspect --url "{{db_url_local}}/{{db}}?sslmode=disable"

# Inspect live database (SQL output)
db-inspect-sql db=default_db:
    atlas schema inspect --url "{{db_url_local}}/{{db}}?sslmode=disable" --format '{{{{sql .}}}}'

# Generate ERD (Mermaid)
db-erd db=default_db:
    atlas schema inspect --url "{{db_url_local}}/{{db}}?sslmode=disable" --format '{{{{mermaid .}}}}' > docs/erd-{{db}}.mmd
    @echo "ERD saved to docs/erd-{{db}}.mmd"

# Check for schema drift
db-drift db=default_db:
    atlas schema diff \
      --from "{{db_url_local}}/{{db}}?sslmode=disable" \
      --to "file://{{migrations_dir}}/{{db}}"

# =============================================================================
# Database Setup
# =============================================================================

# Create mydatabase
db-create:
    @echo "Creating mydatabase..."
    docker exec dockers-postgres-1 psql -U myuser -d postgres -c "CREATE DATABASE mydatabase;" 2>/dev/null || true
    @echo "Database created!"

# Reset database (DESTRUCTIVE)
db-reset db=default_db:
    @echo "Resetting {{db}} database..."
    docker exec dockers-postgres-1 psql -U myuser -d postgres -c "DROP DATABASE IF EXISTS {{db}};"
    docker exec dockers-postgres-1 psql -U myuser -d postgres -c "CREATE DATABASE {{db}};"
    just migrate {{db}}

# Full schema workflow: edit HCL → generate migration → apply
schema-push db=default_db name="schema_update":
    @echo "1. Validating schema..."
    just schema-validate {{db}}
    @echo "2. Generating migration..."
    just migrate-diff {{db}} {{name}}
    @echo "3. Applying migration..."
    just migrate {{db}}
    @echo "Done! Schema changes applied."

sort-deps:
    cargo fmt
    cargo sort --workspace

# docker rm $(docker ps -aq) -f
test-all:
    cargo nextest run --workspace

# Full reset: down, prune, up, migrate
reset-db:
    just docker-down
    docker volume prune -af
    docker compose -f manifests/dockers/compose.yaml up -d
    @sleep 3
    just db-create
    just migrate-all

# Start local dev (docker-compose + migrations + apps)
dev:
    mprocs -c manifests/mprocs/local.yaml

# Start Kind dev (port-forward + tilt)
dev-kind:
    mprocs -c manifests/mprocs/kind.yaml

# Start infrastructure + migrations only
dev-infra:
    docker compose -f manifests/dockers/compose.yaml up -d
    @echo "Waiting for PostgreSQL..."
    @until docker exec dockers-postgres-1 pg_isready -U myuser -q 2>/dev/null; do sleep 1; done
    just db-create
    just migrate-all

kompose:
    kubectl create ns dbs
    kompose convert --file ~/private/nx-playground/manifests/dockers/compose.yaml --namespace dbs --stdout | kubectl apply -f -
    just migrate-all-cluster

# Proto/gRPC workflow (using buf)
# Directory containing buf configuration

proto_dir := "manifests/grpc"

# Format proto files
proto-fmt:
    cd {{ proto_dir }} && buf format -w

# Lint proto files
proto-lint:
    cd {{ proto_dir }} && buf lint

# Check for breaking changes (against git main branch)
proto-breaking:
    cd {{ proto_dir }} && buf breaking --against '.git#branch=main'

# Build/validate proto files
proto-build:
    cd {{ proto_dir }} && buf build

# Generate Rust code from proto files
proto-gen:
    cd {{ proto_dir }} && buf generate

# Verify generated Rust code compiles
proto-check:
    cargo check -p rpc

# Full proto workflow: format, lint, build, generate, verify
proto: proto-fmt proto-lint proto-build proto-gen proto-check
    @echo "Proto workflow complete"

# Alias for backward compatibility
buf: proto

# Benchmark tasks API endpoints with wrk
# Directory containing wrk scripts

wrk_dir := "scripts/wrk"
api_url_local := "http://localhost:8080/api"
api_url_cluster := "http://localhost:5221/api"

# ============================================================================
# Local Benchmarks (localhost:8080)
# ============================================================================

# Benchmark GET /api/tasks (gRPC endpoint) - Local
bench-tasks-grpc:
    @echo "=== Benchmarking gRPC Tasks Endpoint (GET) - Local ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/report.lua {{ api_url_local }}/tasks

# Benchmark GET /api/tasks-direct (Direct DB endpoint) - Local
bench-tasks-direct:
    @echo "=== Benchmarking Direct DB Tasks Endpoint (GET) - Local ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/report.lua {{ api_url_local }}/tasks-direct

# Benchmark POST /api/tasks (gRPC endpoint) - Local
bench-tasks-grpc-post:
    @echo "=== Benchmarking gRPC Tasks Endpoint (POST) - Local ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/post-task.lua {{ api_url_local }}/tasks

# Benchmark POST /api/tasks-direct (Direct DB endpoint) - Local
bench-tasks-direct-post:
    @echo "=== Benchmarking Direct DB Tasks Endpoint (POST) - Local ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/post-task.lua {{ api_url_local }}/tasks-direct

# Run all local benchmarks and compare
bench-tasks-compare:
    @echo "======================================"
    @echo "  Tasks API Benchmark Comparison (Local)"
    @echo "======================================"
    @echo ""
    just bench-tasks-grpc
    @echo ""
    just bench-tasks-direct
    @echo ""
    @echo "======================================"
    @echo "  POST Benchmarks"
    @echo "======================================"
    @echo ""
    just bench-tasks-grpc-post
    @echo ""
    just bench-tasks-direct-post

# ============================================================================
# Cluster Benchmarks (Kind via Tilt port-forward on localhost:5221)
# ============================================================================

# Benchmark GET /api/tasks (gRPC endpoint) - Cluster
bench-cluster-tasks-grpc:
    @echo "=== Benchmarking gRPC Tasks Endpoint (GET) - Cluster ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/report.lua {{ api_url_cluster }}/tasks

# Benchmark GET /api/tasks-direct (Direct DB endpoint) - Cluster
bench-cluster-tasks-direct:
    @echo "=== Benchmarking Direct DB Tasks Endpoint (GET) - Cluster ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/report.lua {{ api_url_cluster }}/tasks-direct

# Benchmark POST /api/tasks (gRPC endpoint) - Cluster
bench-cluster-tasks-grpc-post:
    @echo "=== Benchmarking gRPC Tasks Endpoint (POST) - Cluster ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/post-task.lua {{ api_url_cluster }}/tasks

# Benchmark POST /api/tasks-direct (Direct DB endpoint) - Cluster
bench-cluster-tasks-direct-post:
    @echo "=== Benchmarking Direct DB Tasks Endpoint (POST) - Cluster ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/post-task.lua {{ api_url_cluster }}/tasks-direct

# Run all cluster benchmarks and compare
bench-cluster-compare:
    @echo "======================================"
    @echo "  Tasks API Benchmark Comparison (Cluster)"
    @echo "======================================"
    @echo ""
    just bench-cluster-tasks-grpc
    @echo ""
    just bench-cluster-tasks-direct
    @echo ""
    @echo "======================================"
    @echo "  POST Benchmarks (Cluster)"
    @echo "======================================"
    @echo ""
    just bench-cluster-tasks-grpc-post
    @echo ""
    just bench-cluster-tasks-direct-post

# Quick cluster benchmark (10s duration, lighter load)
bench-cluster-quick:
    @echo "=== Quick Benchmark: gRPC GET (Cluster) ==="
    wrk -t2 -c10 -d10s --latency {{ api_url_cluster }}/tasks
    @echo ""
    @echo "=== Quick Benchmark: Direct DB GET (Cluster) ==="
    wrk -t2 -c10 -d10s --latency {{ api_url_cluster }}/tasks-direct
    @echo ""
    @echo "Benchmark complete!"

# Quick benchmark (10s duration, lighter load) - Local
bench-tasks-quick:
    @echo "=== Quick Benchmark: gRPC GET (Local) ==="
    wrk -t2 -c10 -d10s --latency {{ api_url_local }}/tasks
    @echo ""
    @echo "=== Quick Benchmark: Direct DB GET (Local) ==="
    wrk -t2 -c10 -d10s --latency {{ api_url_local }}/tasks-direct

backstage-dev:
    kubectl apply -k manifests/kustomize/backstage/overlays/dev

backstage-prod:
    kubectl apply -k manifests/kustomize/backstage/overlays/prod

backstage-logs:
    kubectl logs -n backstage deployment/backstage -f

backstage-catalog-generate:
    nu scripts/nu/generate-backstage-catalog.nu

crossplane-functions-install:
    echo 'apiVersion: pkg.crossplane.io/v1beta1\nkind: Function\nmetadata:\n  name: function-kcl\nspec:\n  package: docker.io/kcllang/function-kcl:latest' | kubectl apply -f -
    echo 'apiVersion: pkg.crossplane.io/v1beta1\nkind: Function\nmetadata:\n  name: function-cue\nspec:\n  package: docker.io/crossplane-contrib/function-cue:latest' | kubectl apply -f -

backstage-setup-github:
    nu scripts/nu/backstage-setup-providers.nu github

backstage-setup-aws:
    nu scripts/nu/backstage-setup-providers.nu aws

backstage-setup-gcp:
    nu scripts/nu/backstage-setup-providers.nu gcp

backstage-setup-cloudflare:
    nu scripts/nu/backstage-setup-providers.nu cloudflare

backstage-setup-all:
    nu scripts/nu/backstage-setup-providers.nu all

backstage-restart:
    kubectl rollout restart deployment/backstage -n backstage
    kubectl rollout status deployment/backstage -n backstage

# ============================================================================
# Local Development Environment
# ============================================================================

# Start full local dev environment (Kind + DBs + Secrets + Tilt)
local-up *args:
    nu scripts/nu/mod.nu up {{args}}

# Tear down local dev environment
local-down *args:
    nu scripts/nu/mod.nu down {{args}}

# Quick restart (keep cluster, redeploy apps)
local-restart:
    nu scripts/nu/mod.nu down --keep-cluster
    tilt up

# Show environment status
local-status:
    nu scripts/nu/mod.nu status

# Generate schema docs from live DB
db-docs db=default_db:
    atlas schema inspect --url "{{db_url_local}}/{{db}}?sslmode=disable" > docs/schema-{{db}}.hcl
    @echo "Schema saved to docs/schema-{{db}}.hcl"

# ============================================================================
# CNPG + Atlas Operator (Kubernetes)
# ============================================================================

# Install CNPG operator (using server-side apply for large CRDs)
cnpg-install:
    kubectl apply --server-side -f https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.24/releases/cnpg-1.24.0.yaml
    @echo "CNPG operator installed. Waiting for it to be ready..."
    kubectl wait --for=condition=available --timeout=120s deployment/cnpg-controller-manager -n cnpg-system

# Install Atlas Kubernetes operator
atlas-operator-install:
    helm install atlas-operator oci://ghcr.io/ariga/charts/atlas-operator --namespace atlas-operator --create-namespace
    @echo "Atlas operator installed"

# Install both operators (CNPG + Atlas) - run this first on a new cluster
operators-install:
    @echo "=== Installing Kubernetes Operators ==="
    @echo ""
    @echo "1. Installing CNPG operator..."
    just cnpg-install
    @echo ""
    @echo "2. Installing Atlas operator..."
    just atlas-operator-install
    @echo ""
    @echo "=== Operators Ready ==="

# Full Kind + CNPG + Atlas setup (from scratch)
# Usage:
#   just kind-cnpg-setup              - Setup with dev environment
#   just kind-cnpg-setup staging      - Setup with staging environment
kind-cnpg-setup env="dev":
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Kind + CNPG + Atlas Full Setup ==="
    echo ""

    # Check if Kind cluster exists
    if ! kind get clusters 2>/dev/null | grep -q "dev"; then
        echo "1. Creating Kind cluster..."
        kind create cluster --name dev
    else
        echo "1. Kind cluster 'dev' already exists"
    fi
    echo ""

    # Install operators (idempotent)
    echo "2. Installing operators..."
    if ! kubectl get deployment cnpg-controller-manager -n cnpg-system &>/dev/null; then
        just cnpg-install
    else
        echo "   CNPG operator already installed"
    fi

    if ! kubectl get deployment -n atlas-operator 2>/dev/null | grep -q atlas; then
        just atlas-operator-install
    else
        echo "   Atlas operator already installed"
    fi
    echo ""

    # Deploy CNPG cluster with migrations
    echo "3. Deploying CNPG cluster ({{env}} environment)..."
    just cnpg-{{env}}
    echo ""

    # Wait for cluster to be ready
    echo "4. Waiting for CNPG cluster to be ready..."
    kubectl wait --for=condition=Ready cluster/dev-mydatabase-db -n mydatabase-{{env}} --timeout=300s 2>/dev/null || \
        echo "   Cluster still initializing (this is normal for first deploy)"
    echo ""

    # Check migration status
    echo "5. Checking migration status..."
    sleep 5  # Give Atlas operator time to pick up the migration
    kubectl get atlasmigration -n mydatabase-{{env}} -o wide 2>/dev/null || echo "   Migration pending..."
    echo ""

    echo "=== Setup Complete ==="
    echo ""
    echo "To access the database:"
    echo "  kubectl port-forward -n mydatabase-{{env}} svc/dev-mydatabase-db-rw 5433:5432"
    echo "  psql 'postgres://mydatabase:dev-password-change-me@localhost:5433/mydatabase'"
    echo ""
    echo "To check status:"
    echo "  just cnpg-status mydatabase-{{env}}"

# Generate schema ConfigMap for AtlasSchema (dev overlay)
gen-schema-configmap:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Generating schema ConfigMap..."
    cat > manifests/k8s/overlays/dev/schema-configmap.yaml << 'HEADER'
    # Schema ConfigMap for AtlasSchema operator
    # Auto-generated by: just gen-schema-configmap
    # Do not edit manually!
    apiVersion: v1
    kind: ConfigMap
    metadata:
      name: mydatabase-schema
      namespace: dbs
      labels:
        app.kubernetes.io/name: mydatabase-schema
        app.kubernetes.io/component: schema
    data:
      schema.sql: |
    HEADER
    sed 's/^/        /' manifests/schemas/schema.sql >> manifests/k8s/overlays/dev/schema-configmap.yaml
    echo "ConfigMap generated at manifests/k8s/overlays/dev/schema-configmap.yaml"

# Generate migrations ConfigMap for Kubernetes
cnpg-gen-migrations:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Generating migrations ConfigMap..."
    cat > manifests/cnpg/base/migrations-configmap.yaml << 'HEADER'
    # Migration ConfigMap for Atlas Operator
    # Auto-generated by: just cnpg-gen-migrations
    # Do not edit manually!
    apiVersion: v1
    kind: ConfigMap
    metadata:
      name: mydatabase-migrations
      labels:
        app.kubernetes.io/name: mydatabase-migrations
        app.kubernetes.io/component: migration
    data:
    HEADER
    # Add each SQL file
    for f in manifests/migrations/mydatabase/*.sql; do
      filename=$(basename "$f")
      echo "  $filename: |" >> manifests/cnpg/base/migrations-configmap.yaml
      sed 's/^/    /' "$f" >> manifests/cnpg/base/migrations-configmap.yaml
    done
    # Add atlas.sum
    echo "  atlas.sum: |" >> manifests/cnpg/base/migrations-configmap.yaml
    sed 's/^/    /' manifests/migrations/mydatabase/atlas.sum >> manifests/cnpg/base/migrations-configmap.yaml
    echo "ConfigMap generated at manifests/cnpg/base/migrations-configmap.yaml"

# Deploy CNPG database (dev environment)
cnpg-dev:
    just cnpg-gen-migrations
    kubectl apply -k manifests/cnpg/overlays/dev

# Deploy CNPG database (staging)
cnpg-staging:
    just cnpg-gen-migrations
    kubectl apply -k manifests/cnpg/overlays/staging

# Deploy CNPG database (prod)
cnpg-prod:
    just cnpg-gen-migrations
    kubectl apply -k manifests/cnpg/overlays/prod

# Check CNPG cluster status
cnpg-status ns="mydatabase":
    kubectl get clusters -n {{ns}}
    kubectl get pods -n {{ns}} -l cnpg.io/cluster
    @echo ""
    @echo "AtlasMigration status:"
    kubectl get atlasmigration -n {{ns}} -o wide 2>/dev/null || echo "No AtlasMigration resources found"

# View CNPG logs
cnpg-logs ns="mydatabase":
    kubectl logs -n {{ns}} -l cnpg.io/cluster --tail=100 -f

# Just how to create a nx repo template
create-nx-project:
  npx create-nx-workspace@latest --e2eTestRunner playwright --unitTestRunner vitest ---aiAgents claude --workspaceType package-based --packageManager bun --ci github --preset @monodon/rust
  bun nx generate @monodon/rust:library --name=rpc --no-interactive
  nx add @nxext/solid
