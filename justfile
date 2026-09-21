#!/usr/bin/env just --justfile

import 'manifests/db/db.just'

import 'scripts/just/rust.just'
import 'scripts/just/docs.just'
import 'apps/zerg/email-nats/email.just'
import 'manifests/grpc/proto.just'
import 'scripts/just/k8s.just'
import 'scripts/just/todo.just'
import 'scripts/just/local-env.just'
import 'scripts/just/platform.just'
import 'scripts/just/web.just'
import 'scripts/just/tilt.just'
import 'scripts/just/zellij.just'
import 'scripts/just/container.just'
import 'scripts/just/k8s-apps.just'
import 'scripts/wrk/bench.just'

# Load .env into every recipe's environment (so `just dev` etc. see REDIS_HOST,
# DATABASE_URL, ... even without direnv). direnv (.envrc) also loads it for shells.

set dotenv-load

default:
    # just fmt
    # just proto-lint
    just -l

slava:
  cargo install --path apps/butler --force
  kopium gitrepositories.source.toolkit.fluxcd.io -A > prometheusrule.rs
  typos
  rumdl
  ls ~/.cargo/bin
  # check what they do and how we use them
  iac-cli
  zesh
  butler
  dashboard-tui
  dashboard-api
  monodocs
  nufmt
  nx-schema
  typos
  rust-gdbgui
  rust-gdb
  rumdl
  rls
  pg-cli
  protoc-gen-rs
lol:
  devkit up --skip-dbs --skip-tilt --gitops --flux --skip-secrets
events:
    kubectl create ns t
    kubectl create deployment bad --image=ngindssx -n t --dry-run=client -o yaml \
      | yq '.spec.template.spec.hostNetwork = true' | kubectl apply -f -
    kubectl get events -A --field-selector reason=FailedCreate -w

[group('dev')]
prepare: fmt-rust proto-lint
    # just -l

[group('dev')]
generate-env:
    devkit secrets fetch -o .env.local

[group('scaffold')]
gen-ci:
    kcl run scripts/kcl/ci/main.k -D config_file=manifests/ci/ci-config.yaml -S githubWorkflow > .github/workflows/generated-ci.yml

# ============================================================================
# Flows — composite gates. Layered:
#   fmt / fmt-check = ALL ecosystems (aggregate the fmt-*/fmt-check-* leaves)
#   lint / test = ALL ecosystems (aggregate the lint-*/test-* leaves)
#   check  = fmt + lint + test + audit                -> the everyday gate
#   verify = check + proto lint + OSV scan            -> run before push
#   fix    = auto-format everything, then verify      -> run when verify whines
#   weekly = deep dep update (runs `just check` itself) + cross-major preview
#
# Adding an ecosystem (go, python, ...): write lint-<eco>/test-<eco> leaves with
# the ecosystem's NATIVE workspace tool, append them to `lint`/`test`. A
# full-workspace Rust gate is never routed through nx: cargo is already a
# workspace orchestrator and one `--workspace` run beats N per-crate
# invocations. The exception is scope, not orchestration:
# `check-rust-affected` lets nx answer "which crates did this diff touch" and
# runs the per-crate lint/test targets for those only (the CI PR path).
# ============================================================================

# Full read-only gate: everything in `check` + proto lint/additivity + Tiltfile
# drift + container/scan target drift + agent-skill adapter drift + OSV scan +
# dependency boundaries + the todo browser e2e (docker + Chromium; see `e2e` in
# scripts/just/web.just).
# `proto-breaking` compares against the LOCAL `main` ref, so it is only as fresh
# as your last fetch, and it is a no-op while you are standing on main. CI's copy
# is the authoritative one (full history, PR-only).
[group('flow')]
verify: check proto-lint proto-breaking openapi-check tilt-check container-check k8s-check tooling-check agents-check scan boundaries e2e
    @echo "verify: all gates passed"

# Auto-format everything (`just fmt`), then run the full gate
[group('flow')]
fix: fmt verify

# Weekly maintenance: paranoid dep update + preview of remaining cross-major bumps
[group('flow')]
weekly: upkg-paranoid outdated

# OSV vulnerability scan of every lockfile (honors osv-scanner.toml ignores)
[group('quality')]
scan:
    osv-scanner --recursive .

# Every ecosystem's native formatter, invoked once: rustfmt + cargo-sort
# (fmt-rust), buf (proto-fmt), biome (fmt-web), rumdl (fmt-docs), typos
# (fmt-spell). typos runs last on purpose — it rewrites words inside files the
# others have already reflowed. Adding an ecosystem = a new fmt-<eco> leaf
# appended here, same rule as `lint`/`test`.
# Format every ecosystem — the ONE formatting entry point
[group('quality')]
fmt: fmt-rust proto-fmt fmt-web fmt-docs fmt-spell

# No web leaf on purpose: `lint-web` is `biome ci .`, which already fails on
# unformatted JS/TS/JSON/CSS, and a second biome process in the same gate would
# only double the wall time. CI's cargo job runs `fmt-check-rust` directly
# because it installs neither rumdl nor typos.
# Read-only formatting gate over every ecosystem (used by `check`)
[group('quality')]
fmt-check: fmt-check-rust proto-fmt-check fmt-check-docs fmt-check-spell

# The allowlist that keeps `--write-changes` from renaming an identifier is
# _typos.toml — read its header before adding a word.
# Fix spelling across every tracked file, not just markdown
[group('quality')]
fmt-spell:
    typos --write-changes

# Fail on a misspelling anywhere in the tree
[group('quality')]
fmt-check-spell:
    typos

# Lint every ecosystem (read-only) — extend with lint-go/lint-py when they exist
# `doc-check` is rustdoc's pass over the same crates (intra-doc links, doc HTML)
# and `deps-unused` is cargo-machete over the manifests: both are read-only
# static gates that only Rust has today, so they ride with `lint-rust` rather
# than growing a third aggregate.
[group('quality')]
lint: lint-rust doc-check deps-unused lint-web

# Test every ecosystem — extend with test-go/test-py when they exist
# `test-doc` is the second Rust leaf on purpose: nextest cannot run doctests,
# so `test-rust` alone leaves every documented example uncompiled.
[group('quality')]
test: test-rust test-doc test-web test-napi

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
# just run zerg-api    just run zerg-tasks
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
    devkit config init
# Show zellij sessions
[group('z')]
z:
  zellij list-sessions --short
  zellij -s nx-playground action query-tab-names
  zellij -s nx-playground action list-panes
  just zj          # attach, or create from manifests/zellij/nx-playground.kdl
  just zj-kill     # destroy the session (compose + kind survive)
  cargo run -p cluster_dashboard --bin cluster-dashboard
  cargo-machete
  cargo hack check --workspace --each-feature --no-dev-deps
