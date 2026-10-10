# Cargo Feature Audit

Audit of how Cargo features are declared, enabled and compiled across the 49
workspace members. Snapshot date: 2026-09-21.

Evidence for every claim below is reproducible: `cargo metadata --no-deps`
for the feature tables and per-dependency selections, a `cfg(feature = "…")`
scan over each crate's `src/`, `tests/` and `benches/`, and the gate commands
in `scripts/tasks/rust.yml` plus the nx-inferred per-crate targets
(`bun nx show project <crate> --json`).

## The shape

40 of 49 crates declare no features at all. The 9 that do are **dependency
isolation gates**: every feature entry is a `dep:x`, none is a
behaviour/capability flag, and no two features are mutually exclusive.
External crate features are owned centrally by `[workspace.dependencies]` in
the root `Cargo.toml`.

That baseline is sound. Every finding in this document is drift, not design.

| Crate | Features | Default |
|---|---|---|
| `contract_tasks` | `orm` | — |
| `core_proc_macros` | `api_resource`, `sea_orm_resource`, `selectable_fields` | `[]` |
| `database` | `all`, `config`, `postgres`, `redis` | `postgres`, `redis` |
| `domain_cloud_resources` | `k8s` | — |
| `email` | `integration`, `sendgrid`, `smtp` | `smtp` |
| `field-selector` | `axum` | `[]` |
| `grpc-client` | `server` | `[]` |
| `messaging` | `nats` | `[]` |
| `test-utils` | `all`, `nats`, `openapi`, `postgres`, `redis` | `postgres` |

## What the gates actually compile

Only two feature configurations are ever built, and neither covers the middle
of the lattice.

| Gate | Command | Feature set compiled |
|---|---|---|
| main / `task check` | `cargo clippy --workspace --all-targets` | **union** of everything any member enables |
| PR (nx affected) | `cargo clippy --package X --all-targets` | that crate's **`default`** only |

No `--all-features`, no `--no-default-features` and no `cargo hack` appears in
`scripts/tasks/flows.yml`, `scripts/tasks/rust.yml` or `.github/workflows`.

Two consequences follow, and both have already bitten:

1. **Workspace unification forces gates on.** In the main gate, a feature that
   *any* member enables is on for *every* crate in that build. `messaging/nats`,
   `contract_tasks/orm` and `database/{postgres,redis,config}` are therefore
   always compiled there, whatever an individual crate requested.
2. **The off-state of a gate is only ever compiled by the per-crate path.**
   `cargo check -p messaging` (default `[]`) is the only command in this repo
   that compiles the NATS-off path of `messaging`. A crate whose `#[cfg]` gates
   and optional deps disagree is invisible to the main gate.

## Findings

### 1. Features nothing ever compiles

| Feature | `cfg` sites | Enabled by | Risk |
|---|---|---|---|
| `field-selector/axum` | 1 — `libs/core/field-selector/src/lib.rs:312`, `mod axum_integration` | **nobody** | `field-selector` is in `PUBLISHED_CRATES` (`scripts/tasks/rust.yml`). Its default is `[]` and `cargo package` builds defaults, so this module is compiled by no gate and no release step — yet crates.io consumers can enable it. |
| `email/integration` | 1 — `libs/notifications/email/tests/integration_test.rs:350`, `mod nats_integration_tests` | **nobody** | A Docker-backed NATS test module that has never run in any gate. |
| `test-utils/all` | 0 | nobody | Pure alias with no consumer. Dead. |

Suggested: delete `test-utils/all`; decide `email/integration` (wire it into a
gate or drop the module); keep `field-selector/axum` but give it compile
coverage — it is public API that cannot be seen breaking.

`database/all` also has 0 `cfg` sites but is a live alias (`zerg_api` enables
it), so it stays.

### 2. Never-varying features (dead switches)

- `messaging/nats` — enabled by **11 of 11** normal consumers
  (`contract_projects`, `domain_projects`, `domain_todo`, `email`, `todo_api`,
  `todo_cli`, `todo_temporal`, `todo_worker`, `zerg_api`, `zerg_email_nats`,
  `zerg_tasks`). The dependency graph is identical with the feature on or off.
  It is retained deliberately as an option on a second backend: the core layer
  is genuinely backend-agnostic (`error.rs`, `job.rs`, `processor.rs` contain no
  `async_nats` or `nats::` reference), and `default = []` means a non-NATS
  consumer links zero NATS crates. The bet is only cheap while a gate compiles
  the off-state — see finding 5.
- `core_proc_macros/sea_orm_resource` — enabled by **4 of 4** normal consumers.
  The sibling features `api_resource` and `selectable_fields` are enabled only
  by dev-dependencies, but the crate is published, so external opt-in justifies
  all three.

These vary for real and should be left alone: `contract_tasks/orm` (`zerg_api`
genuinely omits it — it does not depend on `domain_tasks`), `grpc-client/server`,
`database/{redis,config}`, `domain_cloud_resources/k8s`, `email/{smtp,sendgrid}`.

### 3. Per-crate feature lists that restate the workspace pin

`tokio` is pinned `features = ['full']` (root `Cargo.toml:152`) and then
re-specified by 7 crates — `butler`, `todo_api`, `todo_cli`, `todo_temporal`,
`todo_web_htmx`, `todo_worker`, `zerg_email_nats` — several as
`["full", "signal"]`, where `full` already contains `signal`, `macros` and
`rt-multi-thread`. `messaging`'s `features = ["sync"]` is inert for the same
reason. Same duplication elsewhere: `uuid` `v4` (`grpc-client`, `messaging`),
`chrono` `serde` and `serde` `derive` (`messaging`), `reqwest` `json` (`email`),
`tracing-subscriber` `env-filter` (`core_config`).

Suggested rule for `AGENTS.md`: *never restate a feature on a
`{ workspace = true }` dependency; add only what the pin lacks.*

Separate decision worth taking: the `tokio = ['full']` pin makes per-crate
tokio slimming impossible for all 30 consumers. If binary size or compile time
starts to matter, drop `full` from the pin and let crates opt in.

### 4. One dependency correct only by unification accident

The `sqlx` pin (root `Cargo.toml:139`) selects no runtime or TLS feature; only
`terran_api` adds `runtime-tokio` + `tls-rustls`. In the workspace gate,
unification hands those to every crate. Verified that `cargo check -p domain_todo`
compiles without them, so there is no hard error today — but the pin does not
express what the workspace needs.

Suggested: move `runtime-tokio`/`tls-rustls` into the pin and drop the per-crate
list. Unrelated but adjacent: `domain_projects` declares `sqlx` and never uses
it (`cargo-machete`); it is `sea-orm` that needs it.

### 5. Highest-value structural fix

Add a non-PR gate that compiles the combinations nothing else does:

```sh
cargo hack check --workspace --each-feature --no-dev-deps
```

It belongs in the weekly/`audit` task rather than the PR path, because it
costs N× compile time. It is the only mechanism that keeps
`field-selector/axum`, `email/integration` and the `messaging/nats` off-path
from rotting silently — and without it, "we keep the feature for a future
backend" is an untested claim rather than a maintained one.

## Clean axes

No action needed on these; they were checked and are healthy.

- Zero `cfg(feature = "…")` references to an undeclared feature across every
  crate's `src/`, `tests/` and `benches/`.
- Zero optional dependencies missing a `dep:` entry, so the workspace has no
  accidental implicit features.
- No mutually exclusive feature pairs, so `--all-features` cannot break.

## Failure modes seen in practice

Two distinct `cargo-machete` false-positive classes and one real one, worth
recognising before deleting a flagged dependency:

| Class | Symptom on removal | Correct action |
|---|---|---|
| Feature selector — `domain_cloud_resources`'s `k8s-openapi`, declared only to select kube's required `v1_33` | `compile_error!` from the dependency itself | `[package.metadata.cargo-machete] ignored` with the reason |
| Optional dep referenced by a feature list — `messaging`'s `eyre`, `tower-http` | `feature X includes dep:Y, but Y is not a dependency` | remove the dependency **and** its `dep:` entry |
| Genuinely dead | builds fine | delete |
