---
name: new-rust-crate
description: Scaffold a new Rust crate (lib or app) in this workspace. Use when adding a domain lib, core lib, contract crate, or a new service/API binary.
---

# New Rust crate

## Placement

- `libs/core/<name>` — cross-cutting infrastructure (config, retry, helpers)
- `libs/domains/<name>` — business domain (entities, repository, service, handlers)
- `libs/contracts/<name>` — shared API types (often with ts-rs bindings)
- `apps/<product>/<name>` — binaries (product = zerg | todo | terran)

## Steps

1. Copy the closest sibling's `Cargo.toml` as the template (e.g.
   `libs/domains/todo/Cargo.toml` for a domain). Rules:
   - every dependency `{ workspace = true }` — versions live ONLY in the root
     `[workspace.dependencies]`; add new deps there first
   - `publish = false` unless it's going to crates.io (then also append to
     `published_crates` in `scripts/just/rust.just`)
2. Add the path to the root `Cargo.toml` `[workspace] members` list — it is an
   explicit list, not a glob; alphabetical within its section
3. Nx discovers the crate automatically via @monodon/rust (verify:
   `bun nx show projects | grep <name>`); no project.json needed unless it
   requires container targets — then copy a sibling's `project.json`
4. Tests: unit tests inline; integration tests in `tests/` using `test-utils`
   (`TestPostgres`/`TestRedis`/`TestNats`) as dev-dependency with the features
   it needs, e.g. `test-utils = { workspace = true, features = ["nats"] }`
5. Gate: `cargo check -p <name>`, then `just check-quick`. Do NOT run the crate
   through `nx run-many` (see AGENTS.md: cargo-native policy).

## Domain crate conventions (libs/domains/*)

Follow `libs/domains/todo`: `models.rs` (+ ts-rs `#[derive(TS)]` exports to
`types/` — generated dir, never hand-edit), `repository.rs`, `service.rs`,
`error.rs` mapping domain errors to `axum_helpers::AppError`, wired via
`impl_into_response_via_app_error!`.
