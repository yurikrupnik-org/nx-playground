#!/usr/bin/env just --justfile

import 'manifests/db/db.just'

default:
    just -l

# =============================================================================
# Rust Quality Checks
# =============================================================================

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

# Sort Cargo.toml dependencies
sort-deps:
    cargo fmt
    cargo sort --workspace

# =============================================================================
# Docker
# =============================================================================

_docker-up:
    docker compose -f manifests/dockers/compose.yaml up -d

# Remove local env db
docker-down:
    docker compose -f manifests/dockers/compose.yaml down

# =============================================================================
# Local Development
# =============================================================================

run *args:
    bacon {{ args }}

# Run zerg web dev server
web:
    cd apps/zerg/web && bun run dev

# Start local dev (docker-compose + migrations + apps)
dev:
    mprocs -c manifests/mprocs/zerg.yaml

# Start Kind dev (port-forward + tilt)
dev-kind:
    mprocs -c manifests/mprocs/kind.yaml

kompose:
    kubectl create ns dbs
    kompose convert --file ~/private/nx-playground/manifests/dockers/compose.yaml --namespace dbs --stdout | kubectl apply -f -
    just migrate-cluster

# =============================================================================
# Proto/gRPC (buf)
# =============================================================================

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

# =============================================================================
# Benchmarks
# =============================================================================

wrk_dir := "scripts/wrk"
api_url_local := "http://localhost:8080/api"
api_url_cluster := "http://localhost:5221/api"

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

# =============================================================================
# Backstage
# =============================================================================

backstage-dev:
    kubectl apply -k manifests/kustomize/backstage/overlays/dev

backstage-prod:
    kubectl apply -k manifests/kustomize/backstage/overlays/prod

backstage-logs:
    kubectl logs -n backstage deployment/backstage -f

backstage-catalog-generate:
    nu scripts/nu/generate-backstage-catalog.nu

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

crossplane-functions-install:
    echo 'apiVersion: pkg.crossplane.io/v1beta1\nkind: Function\nmetadata:\n  name: function-kcl\nspec:\n  package: docker.io/kcllang/function-kcl:latest' | kubectl apply -f -
    echo 'apiVersion: pkg.crossplane.io/v1beta1\nkind: Function\nmetadata:\n  name: function-cue\nspec:\n  package: docker.io/crossplane-contrib/function-cue:latest' | kubectl apply -f -

# =============================================================================
# K8s Operators
# =============================================================================

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
kind-cnpg-setup env="dev":
    #!/usr/bin/env bash
    set -euo pipefail

    echo "=== Kind + CNPG + Atlas Full Setup ==="
    echo ""

    if ! kind get clusters 2>/dev/null | grep -q "dev"; then
        echo "1. Creating Kind cluster..."
        kind create cluster --name dev
    else
        echo "1. Kind cluster 'dev' already exists"
    fi
    echo ""

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

    echo "3. Deploying CNPG cluster ({{ env }} environment)..."
    just cnpg-deploy {{ env }}
    echo ""

    echo "4. Waiting for CNPG cluster to be ready..."
    kubectl wait --for=condition=Ready cluster/dev-mydatabase-db -n mydatabase-{{ env }} --timeout=300s 2>/dev/null || \
        echo "   Cluster still initializing (this is normal for first deploy)"
    echo ""

    echo "5. Checking migration status..."
    sleep 5
    kubectl get atlasmigration -n mydatabase-{{ env }} -o wide 2>/dev/null || echo "   Migration pending..."
    echo ""

    echo "=== Setup Complete ==="
    echo ""
    echo "To access the database:"
    echo "  kubectl port-forward -n mydatabase-{{ env }} svc/dev-mydatabase-db-rw 5433:5432"
    echo "  psql 'postgres://mydatabase:dev-password-change-me@localhost:5433/mydatabase'"
    echo ""
    echo "To check status:"
    echo "  just cnpg-status mydatabase-{{ env }}"

# =============================================================================
# Local Environment (Kind + Tilt)
# =============================================================================

# Start full local dev environment (Kind + DBs + Secrets + Tilt)
local-up *args:
    nu scripts/nu/mod.nu up {{ args }}

# Tear down local dev environment
local-down *args:
    nu scripts/nu/mod.nu down {{ args }}

# Quick restart (keep cluster, redeploy apps)
local-restart:
    nu scripts/nu/mod.nu down --keep-cluster
    tilt up

# Show environment status
local-status:
    nu scripts/nu/mod.nu status

# =============================================================================
# Misc
# =============================================================================

# Just how to create a nx repo template
create-nx-project:
  npx create-nx-workspace@latest --e2eTestRunner playwright --unitTestRunner vitest ---aiAgents claude --workspaceType package-based --packageManager bun --ci github --preset @monodon/rust
  bun nx generate @monodon/rust:library --name=rpc --no-interactive
  nx add @nxext/solid
