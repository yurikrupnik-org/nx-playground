# Rust workspace recipes — imported by the root justfile.
# Everything here runs cargo's own workspace orchestration (one --workspace
# invocation, shared target dir). Rust is deliberately NOT routed through nx;
# see the benchmark note in the root justfile header.

# Clippy on all crates, warnings are errors
[group('quality')]
lint-rust:
    cargo clippy --workspace --all-targets -- -D warnings

# nextest across the cargo workspace
[group('quality')]
test-rust:
    cargo nextest run --workspace

# Check formatting without modifying files
[group('quality')]
fmt-check:
    cargo fmt --all --check

# Format all Rust code
[group('quality')]
fmt:
    cargo fmt --all

# Format + sort Cargo.toml dependency tables
[group('quality')]
sort-deps: fmt
    cargo sort --workspace

# Security and dependency checks
[group('quality')]
audit:
    # RUSTSEC-2023-0071: RSA timing vulnerability - no fix available
    # RUSTSEC-2026-0235: rkyv 0.7 — lockfile-only optional dep of rust_decimal,
    #   never compiled (cargo tree -i rkyv: not in graph); no semver-compatible fix
    cargo audit --ignore RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0235
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
