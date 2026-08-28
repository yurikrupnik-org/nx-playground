# Agent notes — nx-playground

Rust + web (Solid/vite) monorepo. Nx manages the project graph/CI/containers;
cargo manages Rust builds. Task runner is `just`.

## Task runner layout

Root `justfile` holds cross-ecosystem flows and aggregates; domain recipes are
imported (flat namespace, `just -l` shows everything):

- `scripts/just/rust.just` — all cargo commands (lint-rust, test-rust, fmt, audit, crates-*)
- `scripts/just/web.just` — nx/biome/ncu (lint-web, test-web, web-fix, outdated-node)
- `scripts/just/platform.just` — DevEnvironment manager (Crossplane): platform-install,
  env-create/status/delete. XRD+KCL composition in `platform/dev-env/`, SDKs in
  `libs/platform/devenv-sdk` (rust) and `platform/sdk/{python,node}` — see `platform/README.md`.
  Node SDK examples must run under `node`, not bun (client-node TLS agent incompatibility).
- `scripts/just/{k8s,local-env}.just`, `manifests/grpc/proto.just`,
  `manifests/db/db.just`, `manifests/kustomize/backstage/backstage.just`,
  `scripts/wrk/bench.just`, `apps/zerg/email-nats/email.just`

Flows: `just check` (everyday gate) · `just verify` (pre-push: check + proto-lint + OSV scan)
· `just fix` (auto-format all, then verify) · `just weekly` (upkg-paranoid + outdated).
Aggregates `lint`/`test` fan out to `lint-rust lint-web` / `test-rust test-web`;
new ecosystems (go/py) add a leaf + append to the aggregate.

## Hard-won rules

- **Never run Rust tasks through nx** (`nx run-many -t test/lint/build` on crates).
  Benchmarked: 3–8x slower than `cargo <cmd> --workspace` — per-crate cargo
  processes serialize on the target-dir lock and nx cache never hits (shared
  `dist/target` isn't fingerprintable). Nx is for web (`-p '*-web'`), affected
  in CI, and container targets.
- **Dependency updates go through `upkg`** (`just upkg` / `upkg-fast` / `upkg-paranoid`),
  never raw `cargo upgrade --incompatible` — it bulldozes range pins and skips
  OSV scans / post-checks. upkg lives in dotconfig (`config/scripts/upkg.nu`).
- **`testcontainers = "=0.27.3"` is exact-pinned** on purpose: latest
  testcontainers-modules (0.15.0) requires ^0.27. The `=` is what makes
  `cargo upgrade` skip it. Unpin when modules supports 0.28
  (check: sparse index deps of testcontainers-modules).
- **Web `lint` scripts are `biome check --write` (mutating)** — gates must use
  `bunx biome ci .` (read-only, root biome.json). `just lint-web` does this.
- **Generated files, do not edit or lint**: `libs/**/types` (ts-rs bindings from
  `export_bindings_*` tests), `libs/rpc/src/generated` (buf), `docs/openapi`
  (OpenAPI v1 specs from `export_openapi_*` tests). biome.json ignores them.
- **Known-unactionable advisories** are ignored in two synced places:
  justfile `audit` recipe (cargo-audit) and `osv-scanner.toml` (OSV). Currently
  RUSTSEC-2023-0071 (rsa, no fix) and RUSTSEC-2026-0235 (rkyv 0.7, lockfile-only
  optional dep of rust_decimal, never compiled).

## Environment gotchas

- `set dotenv-load` in the root justfile injects `.env`/`.env.local` into every
  recipe. `.env` contains an **empty** `SOCKET_CLI_API_TOKEN=` — a non-empty
  env var overrides `socket login`'s stored token and can cause
  "Organization not found" (org is `yuri`; token must be created for it).
- Secrets come from vals + GCP Secret Manager (`just run`, `just dev` wrap with
  `vals exec -i`). Never write plaintext secrets; `.claude/hooks/guard-secrets.js`
  guards this.
- `devkit` (external CLI from ~/dotconfig) handles compose/kind; `scripts/nu/`
  is an older **diverged fork** of the same engine used by `just local-*` —
  do not assume they behave identically.

## Testing

- `cargo nextest run --workspace` — 351 tests, ~90s warm; many use testcontainers
  (docker must be running). `test-utils` provides TestPostgres/TestRedis/TestNats
  wrappers around testcontainers-modules.
- Web: `just test-web` (nx build ×3 apps + vitest where present).
