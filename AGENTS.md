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

Flows: `just check` (everyday gate) · `just verify` (pre-push: check + proto-lint
+ Tiltfile/container-target/k8s-manifest drift + OSV scan) · `just fix`
(auto-format all, then verify) · `just weekly` (upkg-paranoid + outdated).
Aggregates `lint`/`test` fan out to `lint-rust lint-web` / `test-rust test-web test-napi`;
new ecosystems (go/py) add a leaf + append to the aggregate.

## Hard-won rules

- **Never run Rust tasks through nx** (`nx run-many -t test/lint/build` on crates).
  Benchmarked: 3–8x slower than `cargo <cmd> --workspace` — per-crate cargo
  processes serialize on the target-dir lock and nx cache never hits (shared
  `dist/target` isn't fingerprintable). Nx is for web (`-p '*-web'`), affected
  in CI, and container targets. The inferred `build`/`run` targets from
  `tools/nx/rust-targets.ts` exist for graph/CI SHAPE, not for running the
  workspace build: `just test-rust`/`lint-rust` stay cargo-direct, and no
  `test`/`lint` targets are inferred for crates.
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
  (OpenAPI v1 specs from `export_openapi_*` tests), and every `Tiltfile`
  (`just tilt-gen`). biome.json ignores the first three.
- **`butler.toml` is the CLI's config, at two levels — root required, per-app optional.** Root
  `butler.toml` owns everything repo-wide (registry, `env`, infra port-forwards,
  shared cluster resources, per-kind image conventions
  (`[imageDefaults.{service,web,node}]`), per-Dockerfile defaults (three
  `[dockerfileTarget]` entries), the per-kind workload shape every app inherits
  (`[workloadDefaults.{service,web,node}]`), the `[k8s]` renderer — the PINNED
  KCL package `oci://docker.io/yurikrupnik/app` at `tag = "0.1.2"`, the per-env
  image tags `[k8s.imageTag]` (`dev = "dev"`, `prod = "main"`), and
  `extraResources` appended verbatim to the generated aggregate kustomization —
  the `[tilt] contextIgnore` churn list (same values as the `rustIgnore` it
  replaced, renamed because the rust and node image contexts share it), and
  `[container] extra` — apps that ship an image but no workload of their own,
  currently `apps/zerg/vector`); an `<app>/butler.toml` — optional, but an app
  without one is not deployed — owns app-local facts only (its `[workload]`, plus
  `[config]` / `[externalSecret]` and any per-env `[env.<env>.*]` override;
  `[tilt]` dev-loop facts — `hostPort` (a number: the forward is composed from it
  and the merged workload `port`, where the old `portForwards = "5253:8080"`
  spelled the container port a second time), `resourceDeps`, `labels`; a
  non-conventional `[image]`). One fact, one file: unknown keys are a
  parse error, and image inputs have exactly TWO tiers in strict precedence —
  the app's `[image]`, else the repo convention
  `[imageDefaults.<service|web|node>]`. `project.json` is no
  longer an image-fact source (butler does not read the `container` target),
  because that target is itself inferred from these same butler.toml facts. The
  root file also locates the workspace (walk-up markers are `butler.toml`, then
  `nx.json`), so the CLI works in repos without nx.
- **ONE registered local nx plugin, `tools/nx/plugin.ts`, FOUR inference
  modules behind it.** nx forks an isolated worker process per REGISTERED
  plugin, so three entries in `nx.json` cost three node boots per graph build
  (measured: 1.45s vs 1.20s user CPU) for identical output. `plugin.ts` braces
  the module globs into one pattern and dispatches on the path;
  `tilt-targets.ts` → `tilt-gen`/`tilt-check`, `rust-targets.ts` → `build`
  (`cargo build --package <crate>`, `production` configuration adds `--release`;
  a crate with no binary gets `cargo check` and no `production`) plus `run` for
  crates that have a binary, `container-targets.ts` → `container`/`scan`,
  `k8s-targets.ts` → `k8s-gen`/`k8s-check`. App-level targets are keyed on
  `apps/**/butler.toml`, because that file is where an app declares its
  `[workload]` — the manifests it used to be keyed on are now that table's
  output. Zero deps (`createNodesV2` needs neither `@nx/devkit` nor
  `@nx/plugin`), and every target MERGES onto an EXISTING graph node — the plugin
  creates none. `@monodon/rust` infers the nodes and the dep edges but ONLY the
  `nx-release-publish` target, which is why ~30 `project.json` files used to
  hand-copy these. project.json targets OVERRIDE inferred ones — a leftover copy
  silently shadows inference — so the hand-written copies were deleted (27 files,
  1347 lines). The modules share `tools/nx/butler-config.ts`: they read the root
  `butler.toml` in TS while butler resolves the same facts in Rust, and `just
  container-check` (`butler container verify --graph`) is the gate that stops the
  two drifting.
- **Tiltfiles and manifests: nx owns the app list, butler owns the content.**
  `tilt-targets.ts` and `k8s-targets.ts` share one predicate — the app declares a
  `[workload]` in its own `butler.toml` and has a recognizable kind
  (`Cargo.toml` → service, `vite.config.ts` → web, `astro.config.mjs` → node — an
  Astro SSR app, a Node process rendering every request, not a static `dist`
  behind nginx) — so `nx show projects --with-target tilt-gen` is the app set,
  identical to `--with-target k8s-gen`, currently 11. It grew by
  `todo-astro-web` (`apps/todo/web-astro`) and `todo_web_htmx`
  (`apps/todo/web-htmx`): web-htmx needed nothing but a workload, being
  already a cargo service, while web-astro needed the new kind plus a Node
  runtime image. Each target shells out to `butler tilt gen --app <dir>` or
  `butler k8s gen --app <dir>`; the generated k8s stanza is the live package
  render (`k8s_yaml(local('kcl run "oci://docker.io/yurikrupnik/app?tag=0.1.2"
  -D env=dev -q', dir='k8s'))`), not a kustomize build.
  `just tilt-gen` runs `nx run-many -t tilt-gen` then writes the root Tiltfile
  with `--root --apps "$(just _tilt-apps)"`, the roots of those same graph nodes,
  so the include list is nx's answer; `just k8s-gen` does the same with
  `just _k8s-apps` for `manifests/k8s/apps/kustomization.yaml`.
- **`nx show projects --with-target scan` IS the image/scan set CI matrixes
  over.** 12 apps: a recognizable kind (service, web or node) AND (a `[workload]`
  in the app's `butler.toml` OR root `butler.toml` `[container] extra`). It grew
  by `todo_api`, `todo_worker` and `todo-web`, then by `todo-astro-web` and
  `todo_web_htmx`, which were deployed but never had a hand-written container
  target — CI picks them up with no workflow edit (`--with-target=scan`).
  Inference also unified two pre-existing drifts: every app now gets the buildx
  registry cache (previously only zerg_api/email-nats/tasks/vector) and
  `trivy --cache-backend memory` (terran_* lacked it). It also fixed a live
  defect: with no `target` option buildx built each Dockerfile's LAST stage, so
  web images shipped `static-web-server` ("static-only, NO /api proxy") while
  every web Deployment injects the `*-proxy-envs` ConfigMap that only the
  `nginx` stage consumes, and zerg-web's prod HTTPRoute sends `/api` straight to
  the pod. The stage is now emitted and verified, resolved from `[image] target`
  → `[imageDefaults.<kind>].target` → `[dockerfileTarget]`; those two tables sit
  at the ROOT of butler.toml, NOT under `[tilt]`, because the `tilt.` prefix is
  what made the stage look dev-only and hid this for as long as it existed.
- **A manifest is never hand-written and never hand-edited.**
  `manifests/k8s/apps/**` (11 rendered apps + the aggregate kustomization),
  every `<app>/k8s/values.yaml` and `values.<env>.yaml`, and every `Tiltfile` are
  generated from `butler.toml` and gated by `just k8s-check` / `just tilt-check`.
  The image reference has exactly ONE home: butler injects it, and declaring
  `image` in any `[workload]` is a hard error — it used to be spelled three ways
  (`yurikrupnik/todo-api:dev`, `yurikrupnik/zerg-api:main`,
  `$REGISTRY/<name>:latest`). Two deltas the collapse decided on purpose: service
  `runAsUser`/`runAsGroup` is **65532** (the rust image's real `USER`; the old
  65534 matched one of three stacked `USER` lines that never applied), and
  `prometheus` defaults to **false** (only three apps serve `/metrics`; the old
  default annotated four whose `/metrics` is a 404).
  `apps/zerg/shared/k8s/kustomize` is the ONE surviving hand-written
  app-adjacent kustomize tree — no app kind, and the root
  `[[tilt.sharedResource]]` references it by path. Dev-only Secret literals moved
  out of the deleted overlays into `manifests/k8s/dev/app-secrets.yaml`, reached
  through `[k8s] extraResources`.
- **Never derive `docker_build(only=…)` from the nx graph.** `@monodon/rust`
  flattens `[dependencies]` and `[dev-dependencies]` into one edge type, so the nx
  closure is wider than the build (measured: zerg_api 21 vs 18, dragging in
  `libs/testing/test-utils` and `libs/core/field-selector`). butler's
  `build_deps`/`transitive_build_deps` exclude `kind=dev`; that is the whole point
  of the split, and it is why the plugin is a façade rather than a TS
  reimplementation. Never hand-widen `only=`.
- **Per-app config is required for a deployed app, its `[tilt]` table is not.**
  `<app>/butler.toml` is where `[workload]` lives, so all 11 deployed apps have
  one; the `[tilt]` block inside it carries only what cannot be derived (host
  port, ordering, labels) and three apps (`todo/api`, `todo/web`, `todo/worker`)
  carry none. Image inputs use strict precedence: the app's `[image]`, else the
  repo convention
  `[imageDefaults.{service,web,node}]` with per-app values derived (crate name /
  `<app>/dist` / the app dir itself for node, image
  `<registry>/<path-below-apps-joined-by->`). A
  single-app repo (no nx, no plugins) puts an `[app]` section in the root file and
  its stanzas are inlined into the root Tiltfile — that is why there are no
  per-language "presets". See `docs/tilt-generators.md`.
- **No Rust `live_update` in Tiltfiles.** `manifests/dockers/rust.Dockerfile`'s
  `rust` stage is `FROM scratch`: syncing sources in rebuilds nothing while Tilt
  reports success. Rust source changes rebuild the image; the tight `only=` list
  is what keeps that cheap.
- **N-API addons live in `libs/native/*`** and are the one Rust exception to the
  no-nx rule: the deliverable is a JS package, so `just test-napi` drives
  `nx run-many -t build test -p '@native/*'` (build outputs `index.js`,
  `index.d.ts`, `*.node` are declared in the project's `project.json` — the
  `dist` targetDefault would cache nothing). They build with the workspace
  `napi` cargo profile (`release` + `panic = 'unwind'`: an aborting panic kills
  the host node process), carry `[lib] test = false` (a cdylib cannot link a
  Rust test harness), and are covered by vitest in `__test__/`.
  **N-API is a Node ABI: an addon cannot load in a browser.** Consume them from
  Node contexts only — astro SSR (`@astrojs/node`), CLIs, tests. Browser reuse
  needs a `wasm32-wasip1` build (napi-rs 3 supports it) plus the
  `@napi-rs/wasm-runtime`/emnapi payload, which is rarely worth it for small pure
  functions. Consumers must also list the package in `vite.ssr.external`: rollup
  cannot inline a `.node` binary. First consumer:
  `todo-astro-web`'s `/api/todos` proxy projects rows through
  `@native/field-selector` (`src/lib/projection.ts`), so JS and the Rust services
  share one field-projection implementation instead of two that drift.
  A node image compiles the addon INSIDE the image
  (`manifests/dockers/node.Dockerfile`: rust builder and node runtime on the
  same Debian trixie, because a cdylib's libc must match), since a host `.node`
  is the wrong platform — and `.dockerignore` now carries `**/*.node` so the
  gitignored `libs/native/field-selector/field-selector.darwin-arm64.node`
  cannot ride into the build context and land beside the in-image binary. The
  runtime tree is pruned to the specifiers the built SSR output actually
  resolves: bun 1.4's isolated linker makes the root `node_modules` a 723 MB
  `.bun` store of symlinks and a later `bun install --production` is a no-op, so
  the builder walks `dist` for bare specifiers and materialises only that
  closure — 5.0 MB for web-astro (`@native/field-selector` + `solid-js`).
- **Known-unactionable advisories** are ignored in two synced places:
  justfile `audit` recipe (cargo-audit) and `osv-scanner.toml` (OSV). Currently
  only RUSTSEC-2026-0235 (rkyv 0.7, lockfile-only optional dep of rust_decimal,
  never compiled). The old rsa ignore (RUSTSEC-2023-0071) was dropped: rsa is no
  longer in `Cargo.lock`.
- **Binaries that link `kube` must install a rustls provider.** `kube` turns on
  rustls' `aws-lc-rs` while `hyper-rustls` turns on `ring`; with both linked
  rustls refuses to guess and *panics at first TLS use*. `terran_api::build_state`
  calls `rustls::crypto::aws_lc_rs::default_provider().install_default()` (the
  backend `jsonwebtoken` already requires). Copy that line into any new binary
  that gains a kube dependency.
- **Crossplane composition functions come from `xpkg.upbound.io`**
  (`platform/functions.yaml`, applied by `just crossplane-functions-install`):
  anonymous in-cluster pulls of `docker.io/kcllang/function-kcl` now fail with
  `401 UNAUTHORIZED`, leaving the Function stuck `Installed=False`.
- **Crossplane 2.x still serves the claim-based XRDs here.** `apiextensions/v1`
  CompositeResourceDefinitions (both `platform/dev-env` and
  `platform/cloud-inventory`) apply with a deprecation warning and default to
  `scope: LegacyCluster`, so claims keep working. Do not add `scope:` to them
  unless migrating to v2 namespaced XRs — that drops the claim API.

## Environment gotchas

- `set dotenv-load` in the root justfile injects `.env`/`.env.local` into every
  recipe. `.env` contains an **empty** `SOCKET_CLI_API_TOKEN=` — a non-empty
  env var overrides `socket login`'s stored token and can cause
  "Organization not found" (org is `yuri`; token must be created for it).
- `devkit secrets fetch -o .env.local` names vars after the GCP secret names, so
  it writes a crates.io token as **`CARGO=<token>`**. Anything that spawns
  `$CARGO` as the cargo binary (napi-cli, and just/nx inject the var) dies with
  `spawn ci… ENOENT`; `libs/native/*/package.json` scripts prefix `CARGO=cargo`
  to neutralise it. Same class of trap as the empty `SOCKET_CLI_API_TOKEN`.
- Secrets come from vals + GCP Secret Manager (`just run`, `just dev` wrap with
  `vals exec -i`). Never write plaintext secrets; `.claude/hooks/guard-secrets.js`
  guards this.
- `devkit` (external CLI from ~/dotconfig) is the ONLY local-env engine: it backs
  `just docker-*` (compose) and `just local-*` (kind). The old `scripts/nu/`
  diverged fork was deleted — it hardcoded another machine's paths.

## Testing

- `cargo nextest run --workspace` — 384 tests, ~60s warm; many use testcontainers
  (docker must be running). `test-utils` provides TestPostgres/TestRedis/TestNats
  wrappers around testcontainers-modules.
- Web: `just test-web` = nx `build test typecheck` for every `*-web` app plus the
  shared `web-auth` lib. `typecheck` (`tsc --noEmit`) is the TS gate — `vite build`
  is not, since esbuild strips types without checking them.
- Shared SPA auth (CSRF, `/auth/me` query, route guard) lives in `libs/ui/web-auth`
  (`@ui/web-auth`) and is consumed by zerg-web and terran-web; its tests defend both.
- **Realtime UI lives in exactly one app: `todo-web`.** Its list is driven by a
  Postgres `NOTIFY` trigger (`todos_notify`) → `domain_todo::db_events` →
  SSE/WebSocket, so it reflects writes from any process (other replicas, worker,
  CLI, `psql`) — not an in-process tee, which only sees its own mutations and is
  the trap this replaced. Never add a second event source for the same data; the
  DB notification is also the cache-invalidation signal. See
  `docs/realtime-todo.md`. Other web apps are intentionally request/response.
- **State management is compared by route in `todo-web`**, not chosen globally:
  `/` (TanStack Query + signals) is the real app, `/xstate` and `/effect` are
  equivalent implementations of the same loop for comparison. Both alternatives
  MUST stay `lazy()`-loaded so `/` never downloads them — this vertical publishes
  shipped-JS numbers (`stack_profiles`). Measured: `/` 46.2 kB gz, `+16.0` for
  xstate, `+57.1` for effect. Neither library has a Solid 2 binding
  (`@xstate/solid` and `@effect-atom/atom-solid` both peer on Solid 1), so each
  route hand-writes a ~10-line bridge. See `docs/todo-state-management.md`.
- **`resolve.conditions` must not include `development` in builds.** All three
  SPAs did, which shipped solid-js's `dist/dev.js` (warnings + debug hooks) to
  production; now gated on `command === 'serve'`. Cost was ~9.6 kB gz per app.
