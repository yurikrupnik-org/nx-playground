#!/usr/bin/env just --justfile

import 'manifests/db/db.just'

# Load .env into every recipe's environment (so `just dev` etc. see REDIS_HOST,
# DATABASE_URL, ... even without direnv). direnv (.envrc) also loads it for shells.
set dotenv-load := true

default:
    just -l

generate-env:
  devkit secrets fetch -o .env.local

gen-ci:
  kcl run scripts/kcl/ci/main.k -D config_file=manifests/ci/ci-config.yaml -S githubWorkflow > .github/workflows/generated-ci.yml
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
    # RUSTSEC-2023-0071: RSA timing vulnerability - no fix available
    # RUSTSEC-2026-0235: rkyv 0.7 — lockfile-only optional dep of rust_decimal,
    #   never compiled (cargo tree -i rkyv: not in graph); no semver-compatible fix
    cargo audit --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0235
    cargo deny check --config .cargo/deny.toml

# The open-source crates published to crates.io. Dependency-ordered publishing is
# handled by cargo itself; this list only decides *what* ships. Everything else in
# the workspace is `publish = false`.
published_crates := "-p core_config -p core_retry -p field-selector -p oidc-auth -p api_resource -p sea_orm_resource -p selectable_fields -p core_proc_macros -p axum-helpers"

# Dry-run the crates.io release: package + verify every publishable crate,
# building dependents against the packaged (not path) versions of their deps.
crates-package:
    cargo package {{published_crates}} --allow-dirty

# Publish to crates.io in dependency order. Needs `cargo login` (or
# CARGO_REGISTRY_TOKEN). Re-running after a partial failure is safe only after
# bumping [workspace.package] version — crates.io versions are immutable.
crates-publish: crates-package
    cargo publish {{published_crates}}

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


# Daily deps refresh (cargo+node+uv): OSV scans, npm cooldown, then build/lint/test
upkg:
    upkg

# Quick bump, no scans/tests — for branches you'll build/test anyway; follow with `just check`
upkg-fast:
    upkg --fast

# Deep audit (safe + cargo-vet + Socket) — before releases; needs one-time `socket login`
upkg-paranoid:
    upkg --paranoid

_docker-up:
    devkit dev up -d

# Remove local env db
docker-down:
    devkit dev down

# Run bacon, layering secrets resolved from vals (GCP Secret Manager) on top of
# the inherited shell env (DATABASE_URL, REDIS_HOST, ... from direnv). `-i` keeps
# the parent env; vals injects the secret keys. No plaintext .env needed.
#   just run zerg-api    just run zerg-tasks
run *args:
    vals exec -i -f manifests/secrets/.vals.yaml -- bacon {{ args }}

# Run zerg web dev server
web:
    cd apps/zerg/web && bun run dev

sort-deps:
    just fmt
    cargo sort --workspace

# docker rm $(docker ps -aq) -f
test-all:
    cargo nextest run --workspace

# Start local dev (bacon apps via mprocs). Wrapped in `vals exec -i` so every
# proc mprocs spawns inherits the vals-resolved secrets (fetched once) on top of
# the shell env — same source as `just run`, no plaintext .env needed.
dev:
  vals exec -i -f manifests/secrets/.vals.yaml -- mprocs -c manifests/mprocs/local.yaml

# Start just the todo group (todo-api + todo-worker + todo-web) via mprocs.
dev-todo:
  mprocs -c manifests/mprocs/todo.yaml

# Start just the terran group (terran-api + terran-web) via mprocs.
dev-terran:
  mprocs -c manifests/mprocs/terran.yaml

# Start just the zerg group via mprocs (same set as `just dev`).
dev-zerg:
  mprocs -c manifests/mprocs/local.yaml
#vals exec -i -f manifests/secrets/.vals.yaml --

# Start Kind dev (port-forward + tilt)
dev-kind:
    mprocs -c manifests/mprocs/kind.yaml

# Generate + apply the shared Secret into the local kind cluster from vals refs.
# `vals eval` resolves each ref+gcpsecrets:// at apply time (no plaintext on disk
# or in git). A RANDOM name is stamped so it won't clobber the kustomize-managed
# `zerg-shared-secrets` during migration — rename once deployments point at it.
#   just kind-secret                   # random name, namespace zerg
#   just kind-secret my-secrets        # explicit name (POSITIONAL, not name=...)
#   just kind-secret my-secrets apps   # explicit name + namespace
kind-secret name=`printf "zerg-secrets-%s" "$(openssl rand -hex 4)"` namespace="zerg":
    @echo "{{ name }}" | grep -Eq '^[a-z0-9]([-a-z0-9]*[a-z0-9])?$' || { echo "✗ invalid Secret name '{{ name }}' — pass it POSITIONALLY (RFC1123: lowercase alphanumeric/-). Use: just kind-secret <name> [namespace]"; exit 1; }
    kubectl create namespace {{ namespace }} --dry-run=client -o yaml | kubectl apply -f -
    vals eval -f manifests/secrets/secret.vals.yaml \
      | sed "s/SECRET_NAME_PLACEHOLDER/{{ name }}/" \
      | kubectl apply -n {{ namespace }} -f -
    @echo "✓ applied Secret '{{ name }}' to namespace '{{ namespace }}' (random name — update refs, then rename)"

kompose:
    kubectl create ns dbs
    kompose convert --file ~/private/nx-playground/manifests/dockers/compose.yaml --namespace dbs --stdout | kubectl apply -f -

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
# Email worker: work-queue vs fan-out check
# See manifests/mprocs/local.yaml and docs/architecture-backlog.md item 0.1
# ============================================================================

# Wipe MailHog AND purge the stream. Purging matters: a freshly created durable
# consumer starts from its DeliverPolicy, so leftover history is replayed and
# pollutes the count.
#
# Clear MailHog and purge the EMAILS stream
email-reset:
    @curl -s -X DELETE http://localhost:8025/api/v1/messages > /dev/null
    @nats stream purge EMAILS -f > /dev/null 2>&1 || true
    @echo "MailHog cleared, EMAILS stream purged"

# Publish exactly ONE welcome-email job onto the EMAILS stream
email-publish-one:
    @cargo run -q -p zerg_email_nats --example publish_test

# Emails actually delivered. One published job must deliver ONCE, no matter how
# many worker replicas are running.
#
# Count emails delivered to MailHog
email-count:
    @curl -s http://localhost:8025/api/v2/messages | jq '.total'

# Durable consumers on EMAILS. A work queue has exactly ONE, shared by every
# replica. One-per-replica means each replica receives every message.
#
# List durable consumers on the EMAILS stream
email-consumers:
    @nats consumer ls EMAILS 2>/dev/null || echo "(nats CLI or EMAILS stream unavailable)"

# Delete every durable consumer on EMAILS. Use before a run: consumers leak on
# each worker restart today, and stale ones distort the count.
#
# Delete all durable consumers on EMAILS
email-consumers-clean:
    #!/usr/bin/env bash
    for c in $(nats consumer ls EMAILS -j 2>/dev/null | jq -r '.[]?'); do
      nats consumer rm EMAILS "$c" -f > /dev/null 2>&1 && echo "  removed $c"
    done

# Count live email workers by probing the health ports local.yaml assigns them
# (8081 = `zerg-email`, 8091/8092 = `email-replica-a`/`-b`, 8093/8094 = ad-hoc).
# The NATS consumer count CANNOT do this: correctly-behaving replicas share one
# durable group, so `consumer ls` returns 1 whether one worker runs or five.
email-workers-live:
    #!/usr/bin/env bash
    n=0
    for p in 8081 8091 8092 8093 8094; do
      if curl -s -o /dev/null --max-time 1 "http://localhost:$p/health"; then
        n=$((n + 1))
      fi
    done
    echo "$n"

# Work-queue assertion. Start `email-replica-a` and `email-replica-b` in
# manifests/mprocs/local.yaml first, then run this.
#
# Asserts three things, all of which must hold:
#   1. >= 2 workers are live      - otherwise "one delivery" is trivially true
#                                   and the gate proves nothing
#   2. exactly 1 consumer group   - replicas SHARE `email-worker`; 2+ groups is
#                                   the per-process-UUID bug (item 0.1)
#   3. exactly 1 delivery         - the job was handled once, not once per replica
#
# Assert one published job delivers exactly once across replicas
email-replica-check:
    #!/usr/bin/env bash
    set -euo pipefail
    workers=$(just email-workers-live)
    if [ "$workers" -lt 2 ]; then
      echo "  FAIL - only $workers email worker(s) live; this gate needs >= 2."
      echo "         With a single worker 'one job, one delivery' is vacuous."
      echo "         Start email-replica-a and email-replica-b (mprocs: press 's')."
      exit 1
    fi
    just email-reset
    groups=$(nats consumer ls EMAILS -j 2>/dev/null | jq 'length')
    just email-publish-one > /dev/null
    sleep 5
    delivered=$(just email-count)
    echo "  workers live      : $workers"
    echo "  consumer groups   : $groups (must be 1 - replicas share one)"
    echo "  jobs published    : 1"
    echo "  emails delivered  : $delivered"
    if [ "$groups" != "1" ]; then
      echo "  FAIL - $groups consumer groups; each replica invented its own name."
      echo "         That is the fan-out bug. See docs/architecture-backlog.md item 0.1"
      exit 1
    fi
    if [ "$delivered" = "1" ]; then
      echo "  PASS - work queue: one job, one delivery across $workers workers"
    else
      echo "  FAIL - fan-out: $delivered deliveries for 1 job (one per consumer)."
      echo "         See docs/architecture-backlog.md item 0.1"
      exit 1
    fi

# Load test. Publishes N distinct jobs and asserts every one is delivered exactly
# once across however many replicas are running. Stronger than email-replica-check:
# one job proves the consumer group is shared, N jobs prove it stays correct under
# concurrency, and per-recipient counting catches a duplicate that a bare total hides.
#
# NOTE: MailHog's /api/v2/messages caps `count` at 250 per request regardless of
# `limit`, so recipient-level checks MUST paginate. `.total` alone is accurate but
# cannot distinguish "1000 delivered" from "500 delivered twice".
#
# Assert N published jobs each deliver exactly once (default 2000)
email-scale-check count="2000":
    #!/usr/bin/env bash
    set -euo pipefail
    workers=$(just email-workers-live)
    if [ "$workers" -lt 2 ]; then
      echo "  FAIL - only $workers email worker(s) live; this gate needs >= 2."
      echo "         Distribution across replicas cannot be measured with one."
      exit 1
    fi
    just email-reset
    groups=$(nats consumer ls EMAILS -j 2>/dev/null | jq 'length')
    started=$(python3 -c 'import time;print(time.time())')
    cargo run -q -p zerg_email_nats --example publish_bulk -- {{count}} | tail -1 | sed 's/^/  /'
    for _ in $(seq 1 600); do
      info=$(nats consumer info EMAILS email-worker -j 2>/dev/null)
      [ "$(echo "$info" | jq '.num_pending')" = "0" ] \
        && [ "$(echo "$info" | jq '.num_ack_pending')" = "0" ] && break
      sleep 1
    done
    ended=$(python3 -c 'import time;print(time.time())')
    python3 - {{count}} "$started" "$ended" "$groups" "$workers" <<'PY'
    import collections, json, sys, urllib.request
    count, started, ended, groups, workers = (
        int(sys.argv[1]), float(sys.argv[2]), float(sys.argv[3]), sys.argv[4], sys.argv[5])
    seen, start = [], 0
    while True:
        d = json.load(urllib.request.urlopen(
            f"http://localhost:8025/api/v2/messages?start={start}&limit=200"))
        items = d.get("items", [])
        if not items:
            break
        seen += [f'{m["To"][0]["Mailbox"]}@{m["To"][0]["Domain"]}' for m in items]
        start += len(items)
        if start >= d["total"]:
            break
    got = collections.Counter(seen)
    expected = {f"scale-{i}@example.com" for i in range(count)}
    missing = expected - set(got)
    dupes = {a: c for a, c in got.items() if c > 1}
    elapsed = ended - started
    print(f"  workers live      : {workers}")
    print(f"  consumer groups   : {groups}")
    print(f"  jobs published    : {count}")
    print(f"  emails delivered  : {len(seen)}")
    print(f"  distinct           : {len(got)}")
    print(f"  duplicated        : {len(dupes)}")
    print(f"  missing           : {len(missing)}")
    print(f"  throughput        : {count/elapsed:.0f} emails/s end-to-end ({elapsed:.1f}s)")
    if missing:
        print(f"  FAIL - {len(missing)} job(s) never delivered, e.g. {sorted(missing)[:3]}")
        raise SystemExit(1)
    if dupes:
        print(f"  FAIL - duplicate delivery, e.g. {list(dupes.items())[:3]}")
        print("         Fan-out regression (item 0.1) or missing idempotency (item 0.2).")
        raise SystemExit(1)
    print(f"  PASS - {count} jobs, {count} deliveries, no duplicates, no loss")
    PY


# ============================================================================
# Local Benchmarks (localhost:8080)
# ============================================================================

# Benchmark GET /api/tasks (gRPC endpoint) - Local
bench-tasks-grpc:
    @echo "=== Benchmarking gRPC Tasks Endpoint (GET) - Local ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/report.lua {{ api_url_local }}/tasks

# Benchmark POST /api/tasks (gRPC endpoint) - Local
bench-tasks-grpc-post:
    @echo "=== Benchmarking gRPC Tasks Endpoint (POST) - Local ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/post-task.lua {{ api_url_local }}/tasks

# Run all local tasks benchmarks (GET + POST)
bench-tasks-all:
    @echo "======================================"
    @echo "  Tasks API Benchmarks (Local)"
    @echo "======================================"
    @echo ""
    just bench-tasks-grpc
    @echo ""
    just bench-tasks-grpc-post

# ============================================================================
# Cluster Benchmarks (Kind via Tilt port-forward on localhost:5221)
# ============================================================================

# Benchmark GET /api/tasks (gRPC endpoint) - Cluster
bench-cluster-tasks-grpc:
    @echo "=== Benchmarking gRPC Tasks Endpoint (GET) - Cluster ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/report.lua {{ api_url_cluster }}/tasks

# Benchmark POST /api/tasks (gRPC endpoint) - Cluster
bench-cluster-tasks-grpc-post:
    @echo "=== Benchmarking gRPC Tasks Endpoint (POST) - Cluster ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/post-task.lua {{ api_url_cluster }}/tasks

# Run all cluster tasks benchmarks (GET + POST)
bench-cluster-all:
    @echo "======================================"
    @echo "  Tasks API Benchmarks (Cluster)"
    @echo "======================================"
    @echo ""
    just bench-cluster-tasks-grpc
    @echo ""
    just bench-cluster-tasks-grpc-post

# Quick cluster benchmark (10s duration, lighter load)
bench-cluster-quick:
    @echo "=== Quick Benchmark: gRPC GET (Cluster) ==="
    wrk -t2 -c10 -d10s --latency {{ api_url_cluster }}/tasks
    @echo ""
    @echo "Benchmark complete!"

# Quick benchmark (10s duration, lighter load) - Local
bench-tasks-quick:
    @echo "=== Quick Benchmark: gRPC GET (Local) ==="
    wrk -t2 -c10 -d10s --latency {{ api_url_local }}/tasks

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

# Just how to create a nx repo template
create-nx-project:
  npx create-nx-workspace@latest --e2eTestRunner playwright --unitTestRunner vitest ---aiAgents claude --workspaceType package-based --packageManager bun --ci github --preset @monodon/rust
  bun nx generate @monodon/rust:library --name=rpc --no-interactive
  nx add @nxext/solid
