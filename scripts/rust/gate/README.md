# `rust_workspace_gate` — the cached whole-workspace Rust gate

Five nx targets wrapping the cargo-direct workspace leaves in
`scripts/tasks/rust.yml`, so that Nx Cloud caches their verdict:

| target | task | cargo command |
|---|---|---|
| `lint-workspace` | `task lint-rust` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test-workspace` | `task test-rust` | `cargo nextest run --workspace` |
| `doc-test-workspace` | `task test-doc` | `cargo test --doc --workspace` |
| `doc-workspace` | `task doc-check` | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` |
| `openapi-workspace` | `task openapi-check` | `cargo test --workspace export_openapi` + `git diff --exit-code -- docs/openapi` |

The command is the `task` invocation, not the cargo line, so a developer running
`task lint-rust` and CI running `lint-workspace` execute the same string —
the property the `ci-optimized.yml` header claims for the whole workflow.

## Why this project exists

`task check-rust-affected` hands over to the whole-workspace gate past `MAX`
crates (default 20), because past that point one shared `--workspace` compile
beats a per-crate fan-out. That handover used to call the tasks **directly**,
which meant it ran outside nx — and therefore outside Nx Cloud.

Measured on one commit, two workflows triggered in the same second
(runs `35605711733` and `35605711835`):

| gate | wall | what executed |
|---|---|---|
| per-crate nx targets (old CI) | **2m31s** | 167 `[remote cache]` hits, zero cargo |
| cargo-direct workspace leaves | **8m56s** | `Compiling proc-macro2 … serde …`, full cold dep closure, no cache |

The AGENTS.md benchmark that chose cargo-direct (`2m13s` vs `4m06s`) compared
two *uncached* runs with a **warm target dir**. In CI the target dir is always
cold and Nx Cloud is always warm, so the ordering inverts. Wrapping the same
single cargo invocation in a cached nx target keeps the cold-cache shared-compile
win *and* makes an unchanged re-run a remote hit.

## Target names are deliberately not `lint`/`test`

`lint-workspace`, not `lint`. A plain `lint` here would be picked up by any
broad `nx run-many -t lint` and fire a whole-workspace clippy from inside a
command meant to lint one project — the exact 10-minute fan-out AGENTS.md
forbids. For the same reason this project does **not** carry `lang:rust`:
`check-rust-affected` selects the per-crate run-many by that tag, and must not
find the workspace gate in it.

## `outputs: []` is load-bearing

cargo writes into the shared, unfingerprintable `dist/target`. A cache entry
here may only ever mean *"these inputs passed"*, never *"an artifact was
restored"* — same rule as the per-crate `lint`/`test` targets.

## The `rustWorkspace` input set — correctness note

`nx.json`'s `rustWorkspace` named input is what makes the cache honest: if a
file can change the gate's verdict and is not in that list, a red workspace
goes green from cache. It is deliberately **coarse** — all of `apps/**` and
`libs/**`, not a `.rs`/`.toml` extension filter — because crates read non-Rust
files through `include_str!`:

```text
libs/ui/todo-theme/todo.css
apps/todo/web-htmx/assets/htmx.min.js
libs/core/oidc-auth/testdata/test_jwks.json, test_key.pem
<crate>/../README.md                      # #![doc = include_str!]
docs/openapi/{terran,todo,zerg}.v1.json   # outside apps/libs — listed explicitly
manifests/db/todo/migrations/*.sql        # outside apps/libs — listed explicitly
```

The cost of coarseness is over-invalidation: a `.tsx` edit under `apps/*/web`
busts this cache. That is never *worse* than the cargo-direct path it replaced
(which had no cache at all), and it is the safe direction to be wrong in.

Verified absent, so deliberately not inputs: no `tonic-build`/`prost-build`
build script (generated gRPC `.rs` is committed, so the `.rs` files *are* the
input), no `.sqlx` offline directory and no `sqlx::query!` macros (no
compile-time database schema).

**If you add an `include_str!`/`include_bytes!` pointing outside `apps/`,
`libs/`, `docs/openapi/` or `manifests/db/`, add it to `rustWorkspace`.** A new
include *inside* those trees needs no edit; a new one outside them is the one
hole this design leaves, and it is why the list ends in explicit paths rather
than a wildcard.
