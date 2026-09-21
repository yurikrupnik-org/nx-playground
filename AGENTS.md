# Agent notes — nx-playground

Rust + web (Solid/vite) monorepo. Nx manages the project graph/CI/containers;
cargo manages Rust builds. Task runner is `just`.

## Task runner layout

Root `justfile` holds cross-ecosystem flows and aggregates; domain recipes are
imported (flat namespace, `just -l` shows everything):

- `scripts/just/rust.just` — all cargo commands (lint-rust, test-rust, test-doc, doc-check, deps-unused, fmt-rust, audit, crates-*)
- `scripts/just/web.just` — nx/biome/ncu (lint-web, test-web, fmt-web, outdated-node)
- `scripts/just/python.just` — uv/ruff/pytest via nx, scoped by TAG not project name
  (`-p tag:lang:python`): lint-py, test-py, fmt-py, fmt-check-py, outdated-py
- `scripts/just/docs.just` — `monodocs` (EXTERNAL: lives in yurikrupnik/wasm-and-k8s,
  published to crates.io, installed by `just docs-install` at the version pinned in
  that file — never vendored here again): docs-html/-api/-open, docs-list, docs-lint,
  plus the rumdl leaves fmt-docs/fmt-check-docs. Renders every project README +
  docs/*.md into one self-contained `dist/docs/index.html`.
  `docs-lint` is NOT in `verify`: 36 of 60 projects have no README, so it is the
  worklist, not yet a gate. Markdown formatting is rumdl's (`.rumdl.toml`), not
  `monodocs fmt`'s — one formatter per file.
- `scripts/just/platform.just` — DevEnvironment manager (Crossplane): platform-install,
  env-create/status/delete. XRD+KCL composition in `platform/dev-env/`, SDKs in
  `libs/platform/devenv-sdk` (rust) and `platform/sdk/{python,node}` — see `platform/README.md`.
  Node SDK examples must run under `node`, not bun (client-node TLS agent incompatibility).
- `scripts/just/{k8s,local-env}.just`, `manifests/grpc/proto.just`,
  `manifests/db/db.just`, `manifests/kustomize/backstage/backstage.just`,
  `scripts/wrk/bench.just`, `apps/zerg/email-nats/email.just`

Flows: `just check` (everyday gate) · `just verify` (pre-push: check + proto-lint

- Tiltfile/container-target/k8s-manifest drift + OSV scan) · `just fix`
(auto-format all, then verify) · `just weekly` (upkg-paranoid + outdated).
Aggregates `fmt`/`fmt-check`/`lint`/`test` fan out to their leaves —
`fmt-rust proto-fmt fmt-web fmt-py fmt-docs fmt-spell` /
`lint-rust doc-check deps-unused lint-web lint-py` /
`test-rust test-doc test-web test-napi test-py`; new ecosystems (go) add a leaf + append to the
aggregate. `just fmt` is the ONLY formatting entry point: rustfmt + cargo-sort,
buf, biome, rumdl (markdown), typos (spelling, whole tree).

## Hard-won rules

- **Never run a WHOLE-WORKSPACE Rust task through nx** (`nx run-many -t lint test
  -p tag:rust`). Re-measured while adding the per-crate targets, warm target dir,
  45 crates / 90 tasks: `just lint-rust` + `just test-rust` (one cargo process
  each) **2m13s** (20s + 1m53s, 488 tests), the same work as nx tasks **4m06s**
  at `--parallel=4` and **10m09s** at `--parallel=1` — per-crate cargo processes
  serialize on the target-dir lock and each re-checks the shared dep closure.
  `just lint-rust`/`test-rust` are therefore cargo-direct one-invocation gates,
  and they are what `just check` and a push to main run.
- **The one Rust path that DOES go through nx is affected-scoped**:
  `just check-rust-affected` (the CI PR path) asks nx which crates a diff touched
  (`nx show projects --affected -p tag:rust`) and runs the inferred per-crate
  `lint` (`cargo clippy --package X --all-targets -- -D warnings`), `test`
  (`cargo nextest run --package X --no-tests=pass`), `doc`, `doc-test` and
  `openapi-gate` targets for exactly those,
  Nx Cloud-cached (measured: 1 crate = 4s at 2/2 cache hits; 3 crates × 2 targets
  = 14.7s cold, 16ms at 6/6 hits). A diff with no crate in it runs no cargo at
  all. Past `max` crates (default **20**) it hands over to the workspace leaves
  instead — `lint-rust test-rust test-doc doc-check openapi-check`, **3m40s**
  flat against ~9s per crate (warm marginals on `domain_users`: clippy 0.4s,
  nextest 0.7s, `cargo test --doc` 2.1s, `cargo doc` 1.5s — doctests reuse the
  dev-profile artifacts nextest just built) — so the
  recipe is bounded by the workspace gate, never a 10-minute fan-out.
  Four things make it correct and they are load-bearing: the `rustGlobals`
  namedInput (Cargo.lock, Cargo.toml, rust-toolchain.toml,
  `.cargo/{config,clippy}.toml`) is in every crate target's `inputs`, which is
  ALSO what makes nx treat a dep bump as touching all 45 crates (`sharedGlobals`
  does NOT drive `affected` — only `{workspaceRoot}/...` globs reachable from a
  target's `inputs` do, via `getImplicitlyTouchedProjects`); `nx affected` has NO
  project filter, it FORWARDS `-p` to the command (`-p tag:rust` arrives as a
  cargo argument and fails every task), so the crate list must come from `nx show
  projects` and be handed to `run-many`; the `rust` tag is withheld from a crate
  whose `lint`/`test` come from a package.json script — the N-API addons, whose
  `test` is vitest and whose gate is `just test-napi`;
  and `test` passes `--no-tests=pass`, because nextest exits 4 on a crate with no
  test binaries, which `--workspace` never hits but a single `--package` often
  does. `lint`/`test` on crates carry `outputs: []` on purpose: cargo writes
  into the unfingerprintable shared `dist/target`, so a cache entry may only mean
  "these inputs passed", never "an artifact was restored" (`build` still inherits
  the JS-shaped `{projectRoot}/dist` from `targetDefaults`, which wins over an
  inferred value — that target stays graph/CI SHAPE only).
- **Dependency updates go through `upkg`** (`just upkg` / `upkg-fast` / `upkg-paranoid`),
  never raw `cargo upgrade --incompatible` — it bulldozes range pins and skips
  OSV scans / post-checks. upkg lives in dotconfig (`config/scripts/upkg.nu`).
- **`testcontainers = "=0.27.3"` is exact-pinned** on purpose: latest
  testcontainers-modules (0.15.0) requires ^0.27. The `=` is what makes
  `cargo upgrade` skip it. Unpin when modules supports 0.28
  (check: sparse index deps of testcontainers-modules).
- **`biome ci` is the JS/TS gate, and it is now an inferred nx target.** biome is
  the only linter and formatter for JS/TS here (no eslint, no prettier) with one
  root `biome.json`, so `lint` on a package.json project is
  `bunx biome ci <projectRoot>` — read-only, contributed by
  `tools/nx/polyglot-targets.ts`. The per-app `"lint": "biome check --write ."`
  scripts and the `project.json` wrappers that called them are GONE: a mutating
  formatter must never answer to a gate's name. `just lint-web` (`biome ci .`)
  stays the whole-tree pass.
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
- **ONE registered local nx plugin, `tools/nx/plugin.ts`, SIX inference
  modules behind it.** nx forks an isolated worker process per REGISTERED
  plugin, so three entries in `nx.json` cost three node boots per graph build
  (measured: 1.45s vs 1.20s user CPU) for identical output. `plugin.ts` braces
  the module globs into one pattern and dispatches on the path;
  `tilt-targets.ts` → `tilt-gen`/`tilt-check`, `rust-targets.ts` → `build`
  (`cargo build --package <crate>`, `production` configuration adds `--release`;
  a crate with no binary gets `cargo check` and no `production`) plus `test`
  and the `rust` tag on every crate, `doc`
  (`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`, because rustdoc resolves
  intra-doc links that neither clippy nor nextest looks at and
  `monodocs build --cargo-doc` ships the result), `doc-test`
  (`cargo test --doc`, ONLY for a crate with a library — nextest has no
  rustdoc harness, so without it every documented example is uncompiled, and
  `cargo test --doc` errors on a bin-only package), `run` for crates that have a
  binary and `install` (`cargo install --path <dir> --locked --force`,
  uncached — the binary lands outside the workspace) for a binary crate whose
  `[package] publish` says its binary leaves the repo, today only `butler`,
  `openapi-targets.ts` → `openapi-gate` for a crate that depends on `utoipa`
  and whose `src/` defines an `export_openapi*` test naming a
  `docs/openapi/*.json` path (`cargo test … export_openapi` then
  `git diff --exit-code` over exactly those documents — the export runs inside
  the normal suite, so drift is invisible until someone diffs the tree),
  `container-targets.ts` → `container`/`scan`,
  `k8s-targets.ts` → `k8s-gen`/`k8s-check`, `polyglot-targets.ts` → `fmt` and
  `lint`, the two names BOTH ecosystems answer to. A crate gets
  `cargo fmt --package` + `cargo sort` and `cargo clippy … -D warnings`; a
  package.json project gets `biome check --write --linter-enabled=false <dir>`
  and `biome ci <dir>`; the three directories that are both
  (`libs/contracts/tasks`, `libs/domains/todo`, `libs/native/field-selector`)
  get the commands COMPOSED under the one name, which is why `lint` does not
  live in `rust-targets.ts`. `lint` is cached with `outputs: []`, `fmt` is
  never cached (it rewrites the tree) and is NOT a leaf of `just fmt` — the
  whole-repo pass stays one cargo and one biome process; these targets are the
  `nx affected -t lint`/`-t fmt` scope.
  App-level targets are keyed on
  `apps/**/butler.toml`, because that file is where an app declares its
  `[workload]` — the manifests it used to be keyed on are now that table's
  output. Zero deps (`createNodesV2` needs neither `@nx/devkit` nor
  `@nx/plugin`), and every target MERGES onto an EXISTING graph node.
  `@monodon/rust` contributes ONLY the `nx-release-publish` target, which is
  why ~30 `project.json` files used to
  hand-copy these — and it infers no `tags` at all, which is why the `rust` tag
  (the only marker of "this node is a cargo crate") is contributed here: a
  plugin's `tags` CONCAT onto an existing node, they do not replace it.
  The plugin
  also contributes a `scope:` tag to every node it touches (declared ownership map
  in `tools/nx/scope-tags.ts` — verticals for `apps/**`, `scope:tasks` for the
  extracted service + its domain, `scope:shared` for libs), enforced by
  `just boundaries` (`tools/nx/check-boundaries.ts` over `nx graph --file`): an
  edge may stay inside its scope or point at `scope:shared`. It runs in `just
  verify` and CI; `.github/CODEOWNERS` mirrors the same map.
  project.json AND package.json scripts OVERRIDE inferred targets — a leftover
  copy silently shadows inference — so the hand-written copies were deleted (27
  files, 1347 lines). The modules share `tools/nx/butler-config.ts`: they read the root
  `butler.toml` in TS while butler resolves the same facts in Rust, and `just
  container-check` (`butler container verify --graph`) is the gate that stops the
  two drifting.
- **The crate graph must not need a cargo subprocess.** `@monodon/rust` reads
  `cargo metadata`, so on a runner that installs bun and nothing else (CI's
  `affected` and `container` jobs) that call fails, it silently contributes
  nothing, and nx rejects the WHOLE graph: "the projects in the following
  directories have no name provided" for all 45 crates, followed by
  `todo-e2e`'s `implicitDependencies` pointing at now-nonexistent
  `todo_api`/`todo_web_htmx`. `plugin.ts` therefore contributes the crate
  `name` (from `[package] name`) and `createDependencies` re-derives the
  crate-to-crate edges by matching dependency KEYS against the workspace's
  crate names — a workspace dependency is declared by name
  (`core_config = { workspace = true }`), so no path arithmetic is involved.
  Verified equal with and without cargo on `PATH`: 113 workspace edges, same
  `zerg_api` dependency list, same `--affected --with-target=scan` answer for a
  `libs/core/config` change. Without the edges `nx affected` would silently
  stop rebuilding the image of an app whose library a diff touched.
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
- **N-API addons live in `libs/native/*`** and are the one crate kind whose nx
  targets are NOT cargo: the deliverable is a JS package, so `just test-napi` drives
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
- **Advisory ignore lists are per-lockfile, and all three root-lock lists are
  now empty.** `cargo audit`, `cargo deny` (`.cargo/deny.toml` `ignore = []`)
  and the root `Cargo.lock` carry no ignores: rkyv (RUSTSEC-2026-0235),
  rustls-pemfile (RUSTSEC-2025-0134) and proc-macro-error2 (RUSTSEC-2026-0173)
  all left the graph, and both scanners report a dead ignore (`unused ignores`,
  `advisory-not-detected`). The only surviving ignores are the two build-time
  proc-macro advisories of `apps/todo/web-leptos`, and they live in
  `apps/todo/web-leptos/osv-scanner.toml` — osv-scanner resolves its config
  per scanned file, so a root config never filters a nested lockfile.
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
- **Every platform tool is a row in `docs/tooling/registry.toml`, gated by
  `just tooling-check`** (in `verify`; `tools/tooling/check-registry.ts`). A row
  states what INSTALLS the tool, what USES it and which gate catches it
  breaking; cited paths must resolve (no line numbers — they rot), an `adopted`
  row with `gate = "none"` must be covered by a declared `[[gap]]`, and
  `paid = true` requires a human `approval` record — an agent never signs up for
  a paid service. This exists because `README.md` listed **Istio + Kiali** as
  prerequisites for months with no control plane, no CRs and no canary anywhere
  in the repo (`docs/tooling/service-mesh.md`): north–south is Gateway API
  `HTTPRoute`s parented to `main-gateway`, a Gateway gitops-v1 owns and this
  repo never defines, and east–west is plain ClusterIP + tower middleware. The
  inert `istio-injection` labels in `manifests/k8s/base/namespace.yaml` stay —
  that file is byte-mirrored from gitops-v1 so Flux's apply is a no-op. Playbook:
  `skill://cncf-manager`.
- **A non-trivial commit is reviewed twice, and `just review-bundle` is what
  both passes read** (`tools/review/bundle.ts` → gitignored
  `dist/review/bundle.md`): the STAGED diff, a class per file, the gates that
  diff implies, the generated output with the command that produces it and a
  proof that can actually FAIL (`just proto-check` is `cargo check -p rpc`, so
  the proof for generated protobuf is `just proto-gen` + `git diff
  --exit-code`), and what the working tree adds on top — because
  `coderabbit review --uncommitted` and `codex exec review --uncommitted`
  transmit the working tree, not the index. It writes NOTHING and exits 1 on a
  secret-shaped staged path or secret material in an added line; `--out` must
  be inside the repo and gitignored. Pass 1 is the model that wrote the code,
  pass 2 is `reviewer` + `security-reviewer` + the authenticated vendor CLIs,
  blind to pass 1; a claim raised by ≥2 components is blocker-eligible.
  Playbook: `skill://precommit-review` (`/precommit`, `/precommit-quick`).
- **A skill is written ONCE and exported to every agent runtime.** The procedure
  lives in `.claude/skills/<name>/SKILL.md` (hand-written, reviewed like code);
  `docs/agents/registry.toml` adds what that file cannot carry — the failure the
  skill prevents, the gate that proves it, the runtimes it targets — and
  `tools/agents/gen.ts` renders every adapter: `.gemini/commands/skill/<name>.toml`
  (`/skill:<name>`), `.gemini/GEMINI.md`, `docs/agents/README.md` and
  `docs/agents/skills.html` (the human explainer, "why we have these skills").
  `just agents-check` (in `verify`) fails on drift, on an evidence path that
  stopped existing, and on a skill dir with no row; `just agents-gen`
  regenerates. Same evidence bar as the tool registry: `gate = "none"` is legal
  only with a declared `[[gap]]` (currently `repo-maintenance`, whose rails ARE
  Claude Code hooks and therefore do not port). Three traps: a Gemini custom
  command injects a file with **`@{path}`**, not the bare `@path` of an
  interactive prompt (a bare one is passed through verbatim and the model
  silently gets no procedure); a hosted agent has no checkout, so Bedrock/Vertex
  get a SHORT instruction (capped at Bedrock's 4000-char `instruction` limit,
  enforced by the generator) plus the SKILL.md as a retrieval document, written
  to gitignored `dist/agents/` by `just agents-export`; and `.gemini/settings.json`
  loads `AGENTS.md` itself via `context.fileName`, so this file is never copied
  into a second always-on context that could drift.
- **`x` (`apps/x/cli`) has no route table — it reads the committed OpenAPI
  documents, and that is the only thing a client can honestly share with these
  servers.** The handlers cannot be reused: four of the six `libs/domains/*`
  crates hard-depend on sea-orm, sqlx and axum, so linking one into a CLI drags
  a Postgres driver into a binary that only speaks HTTP. What IS shared is the
  contract, and it is generated from the same `#[utoipa::path]` annotations
  that document the live `/api-docs/openapi.json` endpoints. The command tree
  is therefore built at RUN TIME from JSON, not declared with `#[derive(Parser)]`:
  a route added to an axum router, annotated, and exported by its
  `export_openapi_*` test appears in `x --help` with no Rust written in the CLI.
  **One committed document per API PROCESS**, not per resource —
  `docs/openapi/{todo,zerg,terran}.v1.json`, each embedded with `include_str!`
  so the binary is self-contained for a remote worker with no checkout; the
  older per-resource `todos.v1.json` / `tasks.v1.json` stay as they are and the
  CLI does not read them. The mapping is mechanical and has exactly one
  judgement call in it: `operationId` is a snake_case handler name so its first
  segment is the verb, `tags[0]` is the resource (the tag is plural like the
  route, the operationId tail is singular — `x get todo` for one verb and
  `x get todos` for another is unmemorable), and `list` folds into `get` so
  arity picks the operation (`x get todos` lists, `x get todos <id>` fetches),
  which is the shape people actually type. Five invariants make the reader in
  `apps/x/cli/src/spec.rs` able to stay small, and they are asserted by a walker
  in `test-utils` (feature `openapi`) next to every `export_openapi_*` test:
  3.1.0, every path key starts with `/`, unique non-empty `operationId`, a
  non-empty `tags`, and a declared path parameter for every `{placeholder}`.
  Two of those were live defects when the CLI was written — `todos.v1.json`
  declared no `{id}` parameter on five operations, and `tasks.v1.json` keyed its
  collection path as `""`. **Two gates stop a stale document**: the inferred
  per-crate `openapi-gate` (`tools/nx/openapi-targets.ts`, the affected/PR
  path — it re-exports and `git diff --exit-code`s exactly the documents that
  crate writes) and `just openapi-check` over the workspace (in `verify` and
  the main-branch CI step). It matters more than a usual generated-file gate: a
  stale document is a CLI that addresses routes the server no longer serves.
  Note `utoipa`'s `nest` is a string concat while axum's `nest` is not, so
  `nest(path="/todos")` over a handler annotated `path = "/"` yields the key
  `/todos/` for a route axum serves at `/todos` — the export normalizes the
  trailing slash, and it must, because axum 0.8 does not redirect.
- **`x ui` is the same binary, the same registry, the same transport.** An arm
  of a delivery-surface benchmark that reimplements the data path measures the
  reimplementation. The TUI browses every operation and executes the ones
  needing no input; for anything with arguments it prints the `x …` command
  line instead of growing an input form that is a worse editor than the shell
  the user is already in. `crossterm` is reached through `ratatui::crossterm`
  and is NOT a direct dependency — two versions in the tree make the event
  enums silently incompatible. The dependency list in `apps/x/cli/Cargo.toml`
  is deliberately short because this binary's size and cold-start time are
  measured quantities in `docs/delivery-surface-assets.md`.
- **`x` reads `securitySchemes`, so an operation's `security(...)` annotation is
  load-bearing for the client, not decoration.** A guarded route answers an
  anonymous request with 401 and nothing else; the only machine-readable way to
  say which credential fixes that is the operation's `security` list, resolved
  against `components.securitySchemes`. `x` therefore refuses the request the
  document says will fail, naming the credential, and sends one of exactly two
  things — `--token`/`$X_TOKEN` as `Authorization: Bearer`, or
  `--session`/`$X_SESSION` as the declared cookie — which are the two ingress
  paths `auth_required` accepts (`libs/core/oidc-auth/src/middleware.rs`,
  `extract_credentials`). `just x-token` mints a Keycloak access token from the
  local realm; `just x-session` logs in through an API's own
  `POST /api/auth/login/password` and prints the session id. An annotation that
  omits `security(...)` on a guarded route does not fail a gate — it degrades
  the CLI to a bare 401, which is exactly how every zerg route looked until the
  44 guarded operations were annotated.
- **`apps/todo/web-leptos` is the ONE crate outside the cargo workspace, and
  that is load-bearing in four places.** Every Rust gate here is a single
  host-target `cargo --workspace` run (`scripts/just/rust.just`), and a Leptos
  CSR crate only compiles for `wasm32-unknown-unknown`, so it is in
  `[workspace] exclude` with its own `Cargo.lock` and its own
  `.cargo/config.toml` pinning the target. Consequences, all of which look like
  untidiness and are not: it is exempt from the `{ workspace = true }`
  dependency rule (an excluded crate cannot resolve workspace deps); its
  advisories are ignored in its OWN `apps/todo/web-leptos/osv-scanner.toml`
  (osv-scanner loads the config sitting next to each scanned lockfile — a root
  one is reported as `unused ignores` here) and NOT mirrored into the justfile
  `audit` list, because `just scan` reads every `Cargo.lock` while
  `cargo audit` reads exactly one that does not contain them; `rust-toolchain.toml`
  carries `targets = ["wasm32-unknown-unknown"]` so every entrypoint installs it
  rather than each developer; and `tools/nx/plugin.ts` reads the exclude list so
  the crate gets neither the `rust` tag nor cargo targets — without that,
  `just check-rust-affected` would run `cargo clippy --package todo_web_leptos`
  from the root and fail on a crate cargo cannot see. Its `build` is
  `trunk build --release` from an explicit `project.json`, because the plugin's
  inferred `cargo build` is wrong for a trunk app.
- **`[profile.release.build-override] strip = false` is not a style choice.**
  `strip = true` on the release profile also strips host-side proc-macro
  dylibs, and on macOS/arm64 that corrupts a large one: `libsqlx_macros.dylib`
  (8.3 MB) comes out with a "mis-aligned LINKEDIT string pool" and rustc cannot
  `dlopen` it, so `cargo build --release` of ANY sqlx-dependent crate — every
  API service here — fails locally. CI never saw it because release builds run
  in Linux containers. Build scripts and proc macros are never shipped, so
  stripping them saved nothing in the first place.
- **`scripts/bench/` is the first thing in this repo that measures bytes.**
  Every kB figure that predates it (`docs/todo-delivery-options.md`,
  `docs/todo-state-management.md`, the `stack_profiles` seed) was hand-measured
  and transcribed, on bases that cannot be compared to each other — raw
  uncompressed transfer in one place, gzipped per-entry closure in another.
  `just bench-assets` fixes the basis (first-render closure = `index.html` plus
  what it references; each file compressed alone, because one file is one
  response; raw / `gzip -9` / `brotli -q 11`, and brotli is mandatory because
  it takes another 18% off a wasm module that gzip cannot). `just bench-ops`
  measures bytes per API call, which is the only number on which a CLI and a
  browser app are comparable. Two traps it exists to prevent, both hit while
  writing it: summing a `dist` directory overstates first render by 2.6× (lazy
  route chunks), and `gzip -9 -c FILE` embeds the filename in the header while
  `gzip -9 -c < FILE` does not, so hand-measured figures ran `len(name)+1`
  bytes high. Results: `docs/delivery-surface-assets.md`.

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
- Python: `just test-py` = nx `build test` (uv wheel + pytest/coverage) for every
  project tagged `lang:python`, `just lint-py`/`fmt-check-py` = ruff check/format.
  `build` is in the leaf for the same reason it is in `test-web`: it is the only
  gate on PACKAGING — remove the README that `[project].readme` declares and
  `nx test` still passes while `nx build` fails in `uv_build.build_sdist`.
  Selection is by TAG, never by project name, so a generated python app is gated
  on day one. These DO go through nx (unlike the Rust leaves): the pytest
  `addopts` are `../../`-relative, so a root-level `uv run pytest` writes
  `coverage/` and `reports/` OUTSIDE the repo — the nx target pins `cwd` to the
  project root. CI runs them in a dedicated `python` job (uv + bun) with NO
  affected arm: unlike `rust` (PR → `check-rust-affected`), every python task is
  a 0s cache hit when nothing changed, so graph-diffing would only add a
  `nx-set-shas` step and save nothing. Before this job existed the only python
  coverage on main was `osv-scanner` reading `uv.lock`.
- Shared SPA auth (CSRF, `/auth/me` query, route guard) lives in `libs/ui/web-auth`
  (`@ui/web-auth`) and is consumed by zerg-web and terran-web; its tests defend both.
- **Realtime UI lives in exactly one app: `todo-web`.** Its list is driven by a
  Postgres `NOTIFY` trigger (`todos_notify`) → `domain_todo::db_events` →
  SSE/WebSocket, so it reflects writes from any process (other replicas, worker,
  CLI, `psql`) — not an in-process tee, which only sees its own mutations and is
  the trap this replaced. Never add a second event source for the same data; the
  DB notification is also the cache-invalidation signal. See
  `docs/realtime-todo.md`. Other web apps are intentionally request/response.
- **`todo_api` is ONE backend with four transports on ONE port** (`:8080`):
  REST, SSE, WebSocket and gRPC (`todo.v1.TodoService`, proto in
  `manifests/grpc/proto/apps/v1/todo.proto`, impl `apps/todo/api/src/grpc.rs`).
  tonic routes are merged into the axum router and `axum::serve` speaks h2c, so
  no second listener, port or `butler.toml` change. Two traps: build from
  `Routes::from(axum::Router::new())` — `Routes::default()` carries an
  `UNIMPLEMENTED` fallback that would swallow unknown HTTP paths — and merge
  AFTER the CORS/`TraceLayer::new_for_http` layers. `Watch` streams the same
  DB-sourced bus as SSE. Never add a separate gRPC binary over the todo tables.
  `docs/todo-delivery-options.md` compares every delivery option with measured
  bytes (JSON vs protobuf: 247 B vs 110 B per todo, 2.2×).
- **Browser e2e lives in `apps/todo/e2e` (`just e2e`, in `verify`, not `check`).**
  Playwright owns the whole stack via `webServer` (docker Postgres → todo-api
  with migrations on its readiness path → vite SPA → built Astro node server →
  axum htmx), on ports 55433/18090/3110/3210/3310 so dev servers are never
  reused. Three traps, all hit: Playwright SIGKILLs a webServer's process
  GROUP and port-checks before launching, so (a) `docker run` needs the
  out-of-group watchdog in `scripts/postgres.sh` or the container outlives the
  run and blocks the next one, (b) never launch a dev server via `bun run` —
  bun puts the child in a new group — call `node_modules/.bin/vite` directly,
  and (c) never use `astro dev` — Astro 7 daemonizes it when `am-i-vibing`
  detects an AI-agent shell; serve `astro build` + `node dist/server/entry.mjs`
  (what ships anyway). Postgres' image runs a socket-only bootstrap server
  first: probe with `psql -h 127.0.0.1 -d todo`, not `pg_isready`.
- **State management is compared by route in `todo-web`**, not chosen globally:
  `/` (TanStack Query + signals) is the real app, `/xstate` and `/effect` are
  equivalent implementations of the same loop for comparison. Both alternatives
  MUST stay `lazy()`-loaded so `/` never downloads them — this vertical publishes
  shipped-JS numbers (`stack_profiles`). Measured: `/` 52.1 kB gz, `+15.6` for
  xstate, `+57.1` for effect — remeasured 2026-09-20 by
  `scripts/bench/assets.sh`, which is also why `/` moved from the 46.2 kB this
  file used to claim: `todo-web` could not build at all (solid-js 2.0.0-rc.4
  declares `@solidjs/signals: ^2.0.0-rc.4`, which floated to rc.9, and rc.9
  dropped internals rc.4 re-exports), and the fix — advancing the whole Solid 2
  RC set to its self-consistent head, rc.9 + router next.26 — costs 5.9 kB gz.
  A prerelease caret on a transitive is the trap; pin the SET, not the
  transitive. Neither library has a Solid 2 binding
  (`@xstate/solid` and `@effect-atom/atom-solid` both peer on Solid 1), so each
  route hand-writes a ~10-line bridge. See `docs/todo-state-management.md`.
- **`resolve.conditions` must not include `development` in builds.** All three
  SPAs did, which shipped solid-js's `dist/dev.js` (warnings + debug hooks) to
  production; now gated on `command === 'serve'`. Cost was ~9.6 kB gz per app.
