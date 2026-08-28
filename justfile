#!/usr/bin/env just --justfile

import 'manifests/db/db.just'

import 'scripts/just/rust.just'
import 'apps/zerg/email-nats/email.just'
import 'manifests/grpc/proto.just'
import 'manifests/kustomize/backstage/backstage.just'
import 'scripts/just/k8s.just'
import 'scripts/just/todo.just'
import 'scripts/just/local-env.just'
import 'scripts/just/platform.just'
import 'scripts/just/web.just'
import 'scripts/wrk/bench.just'

# Load .env into every recipe's environment (so `just dev` etc. see REDIS_HOST,
# DATABASE_URL, ... even without direnv). direnv (.envrc) also loads it for shells.

set dotenv-load := true

default:
    just -l

[group('dev')]
generate-env:
  devkit secrets fetch -o .env.local

[group('scaffold')]
gen-ci:
  kcl run scripts/kcl/ci/main.k -D config_file=manifests/ci/ci-config.yaml -S githubWorkflow > .github/workflows/generated-ci.yml

# ============================================================================
# Flows — composite gates. Layered:
#   lint / test = ALL ecosystems (aggregate the lint-*/test-* leaves)
#   check  = fmt + lint + test + audit                -> the everyday gate
#   verify = check + proto lint + OSV scan            -> run before push
#   fix    = auto-format everything, then verify      -> run when verify whines
#   weekly = deep dep update (runs `just check` itself) + cross-major preview
#
# Adding an ecosystem (go, python, ...): write lint-<eco>/test-<eco> leaves with
# the ecosystem's NATIVE workspace tool, append them to `lint`/`test`. Rust is
# deliberately NOT routed through nx: cargo is already a workspace orchestrator
# and one `--workspace` run beats N per-crate invocations.
# ============================================================================

# Full read-only gate: everything in `check` + proto lint + OSV scan — run before push
[group('flow')]
verify: check proto-lint scan
    @echo "verify: all gates passed"

# Auto-fix formatting (rust fmt + Cargo.toml sort + proto + web), then run the full gate
[group('flow')]
fix: sort-deps proto-fmt web-fix verify

# Weekly maintenance: paranoid dep update + preview of remaining cross-major bumps
[group('flow')]
weekly: upkg-paranoid outdated

# OSV vulnerability scan of every lockfile (honors osv-scanner.toml ignores)
[group('quality')]
scan:
    osv-scanner --recursive .

# Lint every ecosystem (read-only) — extend with lint-go/lint-py when they exist
[group('quality')]
lint: lint-rust lint-web

# Test every ecosystem — extend with test-go/test-py when they exist
[group('quality')]
test: test-rust test-web

# The everyday gate: formatting + all linters + all tests + supply-chain audit
[group('quality')]
check: fmt-check lint test audit
    @echo "All checks passed!"

# Quick gate (no tests/audit): formatting + every linter
[group('quality')]
check-quick: fmt-check lint

# To actually update, use `just upkg` (safe) — the raw `cargo update`/`cargo upgrade`
# recipes were removed: upkg runs both WITH scans, pin-respect, and post-checks.
# Preview what a deps refresh would change (read-only), every ecosystem
[group('deps')]
outdated: outdated-rust outdated-node

# Daily deps refresh (cargo+node+uv): OSV scans, npm cooldown, then build/lint/test
[group('deps')]
upkg:
    upkg

# Quick bump, no scans/tests — for branches you'll build/test anyway; follow with `just check`
[group('deps')]
upkg-fast:
    upkg --fast

# Deep audit (safe + cargo-vet + Socket) — before releases; needs one-time `socket login`
[group('deps')]
upkg-paranoid:
    upkg --paranoid

# Run bacon, layering secrets resolved from vals (GCP Secret Manager) on top of
# the inherited shell env (DATABASE_URL, REDIS_HOST, ... from direnv). `-i` keeps
# the parent env; vals injects the secret keys. No plaintext .env needed.
#   just run zerg-api    just run zerg-tasks
[group('dev')]
run *args:
    vals exec -i -f manifests/secrets/.vals.yaml -- bacon {{ args }}

# Start local dev (bacon apps via mprocs). Wrapped in `vals exec -i` so every
# proc mprocs spawns inherits the vals-resolved secrets (fetched once) on top of
# the shell env — same source as `just run`, no plaintext .env needed.
[group('dev')]
dev:
  vals exec -i -f manifests/secrets/.vals.yaml -- mprocs -c manifests/mprocs/local.yaml

# Start just the todo group (todo-api + todo-worker + todo-web) via mprocs.
[group('dev')]
dev-todo:
  mprocs -c manifests/mprocs/todo.yaml

# Start just the terran group (terran-api + terran-web) via mprocs.
[group('dev')]
dev-terran:
  mprocs -c manifests/mprocs/terran.yaml

# Start just the zerg group via mprocs (same set as `just dev`).
[group('dev')]
dev-zerg:
  mprocs -c manifests/mprocs/local.yaml

#vals exec -i -f manifests/secrets/.vals.yaml --

# Start Kind dev (port-forward + tilt)
[group('dev')]
dev-kind:
    mprocs -c manifests/mprocs/kind.yaml

# Proto/gRPC workflow (using buf)
# Directory containing buf configuration

# Benchmark tasks API endpoints with wrk
# Directory containing wrk scripts

# ============================================================================
# Email worker: work-queue vs fan-out check
# See manifests/mprocs/local.yaml and docs/architecture-backlog.md item 0.1
# ============================================================================

# ============================================================================
# Local Benchmarks (localhost:8080)
# ============================================================================

# ============================================================================
# Cluster Benchmarks (Kind via Tilt port-forward on localhost:5221)
# ============================================================================

# ============================================================================
# Local Development Environment
# ============================================================================

# ============================================================================
# CNPG + Atlas Operator (Kubernetes)
# ============================================================================

# Just how to create a nx repo template
[group('scaffold')]
create-nx-project:
  npx create-nx-workspace@latest --e2eTestRunner playwright --unitTestRunner vitest ---aiAgents claude --workspaceType package-based --packageManager bun --ci github --preset @monodon/rust
  bun nx generate @monodon/rust:library --name=rpc --no-interactive
  nx add @nxext/solid

