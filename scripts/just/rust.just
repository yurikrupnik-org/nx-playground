# Rust workspace recipes — imported by the root justfile.
# The full gates (`lint-rust`, `test-rust`) run cargo's own workspace
# orchestration: one --workspace invocation, shared target dir, every shared
# dependency compiled once. They are what `just check` and a push to main run.
# `check-rust-affected` is the other scope — the crates a diff touched — and it
# is the only Rust path that goes through nx; see the benchmark note in the root
# justfile header and the header of tools/nx/rust-targets.ts.

# Clippy on all crates, warnings are errors
[group('quality')]
lint-rust:
    cargo clippy --workspace --all-targets -- -D warnings

# nextest across the cargo workspace
[group('quality')]
test-rust:
    cargo nextest run --workspace

# nextest has no rustdoc harness, so `test-rust` above runs ZERO of the ~210
# doctests in this workspace: `--doc` is a separate cargo invocation or the
# examples never compile. Warm, after `test-rust`, this is ~30s.
# Doctests across the cargo workspace (what nextest cannot run)
[group('quality')]
test-doc:
    cargo test --doc --workspace

# rustdoc is the third front-end over these sources — it resolves intra-doc
# links and `#[doc]` attributes that neither clippy nor nextest looks at, and
# `monodocs build --cargo-doc` ships whatever it produces. `-D warnings` is
# what makes that a gate; `--no-deps` keeps it to this workspace's crates.
# Rustdoc gate: every crate documents cleanly, warnings are errors
[group('quality')]
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace

# Dependency hygiene: a dependency nothing references still compiles, still
# lands in Cargo.lock, still gets audited and still slows every build. Source
# heuristic, no compilation — the whole workspace in under a second. A false
# positive (a crate used only through a macro expansion) goes in that crate's
# `[package.metadata.cargo-machete] ignored`, with the reason, not in an
# allow-list here.
[group('quality')]
deps-unused:
    cargo machete

# Clippy, nextest, doctests, rustdoc and the OpenAPI drift gate for exactly the
# crates a diff touches, plus their cargo dependents (`@monodon/rust` puts
# those edges in the graph). Runs the inferred per-crate targets, so Nx Cloud
# caches a crate's result and a rerun re-checks only what moved; a diff with no
# crate in it runs no cargo at all. This is the CI PR path.
#
# `max` is where per-crate orchestration stops paying, measured on this
# workspace (warm target dir, 45 crates / 90 tasks):
#
#   lint-rust + test-rust + test-doc + doc-check (one cargo process each)
#                                       20s + 1m53s + 31s + 56s = 3m40s
#   all 45 crates through nx, --parallel=4                       4m06s
#   all 45 crates through nx, --parallel=1                      10m09s
#   3 crates through nx, cold / 100% Nx Cloud cache hit       14.7s / 16ms
#
# Marginal cost of the two doc targets, one crate, everything warm
# (`domain_users`): clippy 0.4s, nextest 0.7s, `cargo test --doc` 2.1s,
# `cargo doc` 1.5s — doctests reuse the dev-profile artifacts nextest just
# built, and only rustdoc pays for a metadata pass of its own.
#
# so ~9s per crate against a flat 3m40s: past ~20 crates the shared compile
# of one --workspace run wins and this recipe hands over to it. A Cargo.lock,
# rust-toolchain.toml or .cargo/*.toml change lands in every crate's `inputs`
# (the `rustGlobals` namedInput) and therefore always takes that branch — which
# is exactly right, since a dep bump does change every crate.
#
# Two steps on purpose: `nx affected` has no project filter — it forwards `-p`
# to the underlying command, where `-p tag:rust` lands as a cargo (or biome)
# argument — so `nx show projects` picks the crates and `run-many` executes
# them. The tag also keeps non-crate projects out: their `lint` is `biome ci`,
# which `just lint-web` already runs over the whole tree.
[group('quality')]
check-rust-affected base=env('NX_BASE', 'origin/main') head=env('NX_HEAD', 'HEAD') max='20':
    #!/usr/bin/env bash
    set -euo pipefail
    affected=$(bun nx show projects --affected -p tag:rust \
        --base="{{ base }}" --head="{{ head }}" --json)
    count=$(jq -r 'length' <<<"$affected")
    if [ "$count" -eq 0 ]; then
        echo "check-rust-affected: no crate affected by {{ base }}..{{ head }}"
        exit 0
    fi
    if [ "$count" -gt {{ max }} ]; then
        echo "check-rust-affected: $count crates affected (> {{ max }}) — running the workspace gate"
        just lint-rust
        just test-rust
        just test-doc
        just doc-check
        just openapi-check
        exit 0
    fi
    crates=$(jq -r 'sort | join(",")' <<<"$affected")
    echo "check-rust-affected: $count crates — $crates"
    # `doc-test` exists only for a crate with a library and `openapi-gate` only
    # for the four that export a document; nx runs each target where it exists
    # and skips the rest.
    bun nx run-many -t lint test doc doc-test openapi-gate -p "$crates" --parallel=4

# CI's cargo job runs this leaf directly instead of the `fmt-check` aggregate,
# so it must stay cargo-only — no bun, rumdl or typos.
# Rust formatting gate: rustfmt + Cargo.toml dependency-table order
[group('quality')]
fmt-check-rust:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --all --check
    # cargo-sort has no --quiet: it narrates two lines per crate (50 crates,
    # 100 lines) whether or not anything is wrong. Hold the output and print it
    # only when the check actually fails.
    if ! out=$(cargo sort --workspace --check --check-format 2>&1); then
        printf '%s\n' "$out"
        exit 1
    fi

# Sorting belongs here rather than in its own recipe: a `cargo fmt` that leaves
# the manifests unsorted just makes `fmt-check-rust` fail on the next line.
# Format Rust and sort every Cargo.toml dependency table
[group('quality')]
fmt-rust:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --all
    # Same noise, same treatment: only the crates cargo-sort REWROTE are worth
    # printing — "is sorted already" for 50 crates is not output, it is scroll.
    if ! out=$(cargo sort --workspace 2>&1); then
        printf '%s\n' "$out"
        exit 1
    fi
    printf '%s\n' "$out" | grep -v 'sorted already' | grep '^Finished' || true

# Regenerate every committed OpenAPI document from the `#[utoipa::path]`
# annotations. Same convention as the ts-rs `export_bindings_*` tests: the
# documents in docs/openapi are generated output and are never hand-edited.
[group('quality')]
openapi-gen:
    cargo test --workspace export_openapi

# Drift gate: an annotation changed but the committed document did not. This
# matters more than a usual generated-file gate because `x` (apps/x/cli) embeds
# these documents at compile time and derives its entire command tree from
# them — a stale document is a CLI that addresses routes the server no longer
# serves. `apps/zerg/api/project.json`'s `openapi-gate` is the same check
# scoped to one crate for the nx-affected path; this one covers all of them.
[group('quality')]
openapi-check: openapi-gen
    git diff --exit-code -- docs/openapi

# Security and dependency checks. No --ignore list: advisories that reach the
# root Cargo.lock get fixed. Per-lockfile OSV ignores (for locks cargo-audit
# never opens, e.g. apps/todo/web-leptos) live in an osv-scanner.toml NEXT TO
# that lock — mirroring them here yields ignores cargo-audit reports as unused.
[group('quality')]
audit:
    cargo audit
    cargo deny check --config .cargo/deny.toml

# Show crates behind their latest versions (read-only preview)
[group('deps')]
outdated-rust:
    cargo outdated --workspace

# The open-source crates published to crates.io. Dependency-ordered publishing is
# handled by cargo itself; this list only decides *what* ships. Everything else in
# the workspace is `publish = false`.
published_crates := "-p core_config -p core_retry -p core_strings -p field-selector -p oidc-auth -p api_resource -p sea_orm_resource -p selectable_fields -p core_proc_macros -p axum-helpers"

# Dry-run the crates.io release: package + verify every publishable crate,
# building dependents against the packaged (not path) versions of their deps.
[group('release')]
crates-package:
    cargo package {{ published_crates }} --allow-dirty

# Publish to crates.io in dependency order. Needs `cargo login` (or
# CARGO_REGISTRY_TOKEN). Re-running after a partial failure is safe only after
# bumping [workspace.package] version — crates.io versions are immutable.
[group('release')]
crates-publish: crates-package
    cargo publish {{ published_crates }}
