# Generated from one config surface: nx owns the lists, butler owns the logic

Three tracks come out of `butler.toml` — every app's `Tiltfile`, every app's nx
`container`/`scan` target, and every app's k8s manifests. Nine pieces, each doing
the one thing it is good at:

| piece | responsibility |
|---|---|
| `tools/nx/plugin.ts` (the repo's ONLY registered nx plugin, **zero deps**) | one `createNodesV2` over one braced glob, dispatching to the four inference modules. One registration, not four: nx forks an isolated worker process per registered plugin, and three entries already measured 1.45s vs 1.20s user CPU per graph build for identical output. |
| `tools/nx/tilt-targets.ts` | decides **which projects are Tilt apps** — an app that declares a `[workload]` in its own `butler.toml` plus a recognizable kind (`Cargo.toml` → service, `vite.config.ts` → web, `astro.config.mjs` → node) — and gives each a cached `tilt-gen` / `tilt-check` target. `nx show projects --with-target tilt-gen` *is* the app set. |
| `tools/nx/k8s-targets.ts` | same predicate, one track over: a cached `k8s-gen` / `k8s-check` target on every app that declares a `[workload]`. `nx show projects --with-target k8s-gen` *is* the manifest set — 11 apps. |
| `tools/nx/container-targets.ts` | gives every **deployable app** a `container` / `scan` target, derived from the same `butler.toml` facts. `nx show projects --with-target scan` *is* the image set CI matrixes over. |
| `tools/nx/rust-targets.ts` | gives every cargo crate its `build` (+`production`), `lint` (clippy), `test` (nextest) and the `rust` tag, plus `run` for a crate with a binary — `@monodon/rust` infers nodes and dep edges but only the `nx-release-publish` target. The `lint`/`test` pair exists for ONE caller, `just check-rust-affected`: nx scopes a diff to crates, cargo still owns the full-workspace gate. |
| `butler tilt gen` (Rust) | derives and renders the Tiltfile content — image inputs, k8s backend, and the tight `docker_build(only=...)` |
| `butler k8s gen` (Rust) | merges the workload payload, injects the facts an app must never hand-type, writes `<app>/k8s/values*.yaml`, and renders them through the pinned KCL package into `manifests/k8s/apps/` |
| `butler container verify` (Rust) | re-resolves the image facts and diffs them against the inferred nx targets, so the TS mirror cannot drift |
| `butler.toml`, two levels | the facts that cannot be derived: the per-kind image conventions `[imageDefaults.{service,web,node}]`, the per-kind workload shape `[workloadDefaults.{service,web,node}]`, and the renderer under `[k8s]` (`package`, `tag`, the per-env `[k8s.imageTag]`, `extraResources`). The node image's `buildArg` is `"APP_DIR"` — the app *directory*, not a `dist` path, because that image installs and builds the app itself: its N-API addon must be compiled for the image's platform, so a locally built `dist/` cannot be copied in |

```bash
just tilt-gen                  # nx run-many -t tilt-gen, then the root Tiltfile
just tilt-gen-app zerg_api     # one app, nx-native and cached
just tilt-check                # drift gate; part of `just verify`
just container-check           # container/scan drift gate; part of `just verify`
just k8s-gen                   # nx run-many -t k8s-gen, then the aggregate kustomization
just k8s-gen-app zerg_api      # one app, nx-native and cached
just k8s-check                 # manifest drift gate; part of `just verify`
nx run zerg_api:tilt-gen       # or straight through nx
nx affected -t tilt-gen        # only apps whose inputs changed
```

## Why the plugin does not generate the content itself

`createProjectGraphAsync()` would be the obvious source for `only=`, and it is
wrong here. `@monodon/rust` flattens `[dependencies]` and `[dev-dependencies]`
into a single edge type, so the nx graph's transitive closure is wider than the
build actually needs:

| app | nx closure | butler `build_deps` closure | nx extras |
|---|---|---|---|
| `zerg_api` | 21 | 18 | `libs/testing/test-utils`, `libs/core/field-selector` |
| `terran_api` | 14 | 12 | same two |
| `todo_api` | 14 | 12 | same two |
| `zerg_tasks` | 14 | 13 | `libs/core/field-selector` |

From `cargo metadata`: `axum-helpers -> test-utils kind=dev`,
`selectable_fields -> field-selector kind=dev`, and seven more. A plugin deriving
`only=` from the nx graph would put the test harness in every service image, so an
edit to `test-utils` would rebuild all of them. Re-deriving kinds in TypeScript
would mean a second graph that can disagree with the first, across two languages.
So the plugin infers *targets*, and the target shells out to butler.

KCL was considered and rejected for the same class of reason: it cannot read the
filesystem, run `cargo metadata`, or parse `Cargo.toml`, so all of the derivation
would collapse back into hand-written config — which is exactly what the deleted
`scripts/kcl/tilt` did (249 config lines, blanket `only=['apps/','libs/']`). KCL
remains the right tool for *k8s manifests*, where the input is structured data
rather than a dependency graph.

## Which projects the plugin picks up

An app qualifies by **declaring a `[workload]` in its own `butler.toml`** and
having a recognizable kind (`Cargo.toml` → service, `vite.config.ts` → web,
`astro.config.mjs` → node, tested in that order), and by already being an nx
project. `tools/nx/plugin.ts` braces `apps/**/butler.toml` into its marker glob
for exactly that reason: that file is where an app declares its workload.
Targets are **merged onto the existing project**, so no new nodes appear in the
graph, and `tilt-gen` and `k8s-gen` land on the same set.

Result today — 11 apps:

```
$ nx show projects --with-target tilt-gen     # identical to --with-target k8s-gen
terran-web terran_api todo-astro-web todo-web todo_api todo_web_htmx todo_worker
zerg-web zerg_api zerg_email_nats zerg_tasks
```

Skipped, correctly: `apps/zerg/shared` (a hand-written kustomize tree with no app
kind, referenced by the root `[[tilt.sharedResource]]`), `todo/cli` +
`todo/temporal` (no workload — not deployables). The `container`/`scan` set is
one app wider — a workload is not required there, see below.

The root `Tiltfile` is a workspace-level artifact (infra port-forwards, shared
resources, one `include()` per app) and this workspace has no root project —
adding one would change `nx affected` semantics for every file. So `just tilt-gen`
reads the same graph and passes the list explicitly:

```bash
butler tilt gen --root --apps "$(just _tilt-apps)"   # roots of nodes having tilt-gen
```

nx therefore decides the include list too; butler never second-guesses it.

## The rule that keeps it collapsed

**One fact, one file.** The root file never names an app; an app file never
repeats a repo-wide default. Where two sources *could* own the same fact, the
generator refuses instead of merging:

| fact | owner | if declared twice |
|---|---|---|
| registry, env, infra port-forwards, shared resources, Dockerfile stage defaults, per-kind workload shape (`[workloadDefaults.<service\|web\|node>]`), the pinned renderer and per-env image tag (`[k8s]`, `[k8s.imageTag]`, `[k8s] extraResources`) | root `butler.toml` | n/a — apps cannot declare them |
| host port forward (`hostPort`), resource ordering (`resourceDeps`), labels | `<app>/butler.toml` `[tilt]` | n/a — the root cannot declare them |
| the k8s workload payload — `[workload]`, `[config]`, `[externalSecret]`, `[env.<env>.*]` | `<app>/butler.toml` | n/a — the root states only the per-kind shape the app narrows |
| dockerfile, context, build-args, image tag, stage | in strict precedence: the app's `<app>/butler.toml` `[image]`, else the repo convention `[imageDefaults.<service\|web\|node>]` | n/a — two tiers, the app wins. `project.json` is no longer a tier: its `container` target is itself *inferred* from these same facts (see below), so reading it back would be circular |

Everything else is *derived*, so it cannot drift:

| derived | from |
|---|---|
| which apps participate | `tools/nx/tilt-targets.ts` and `tools/nx/k8s-targets.ts`: a `[workload]` in `<app>/butler.toml` + one of the three recognizable kinds (`Cargo.toml` / `vite.config.ts` / `astro.config.mjs`) — no app registry, no per-app opt-in |
| image inputs for an app that declares none | `[imageDefaults.<kind>]`: dockerfile + stage from the convention, the one per-app build arg derived (crate name for a service, `<app>/dist` for web, the app directory itself for node — `APP_DIR=apps/todo/web-astro`, not a dist path, because the node image installs and builds the app internally so its N-API addon is compiled for the image's own platform), reference `<registry>/<path below apps/ joined by ->` — `apps/todo/api` → `yurikrupnik/todo-api`, which is exactly what its Deployment already pulls |
| app kind (service, web or node) | `Cargo.toml`, else `vite.config.ts`, else `astro.config.mjs` on disk — in that order, so a crate wins over a Vite config, which wins over an Astro one |
| k8s backend | a `[workload]` → the pinned package rendered live over the generated values, `k8s_yaml(local('kcl run "oci://docker.io/yurikrupnik/app?tag=0.1.2" -D env=dev -q', dir='k8s'))`; no workload (a standalone repo, say) → `<app>/k8s/kcl.mod` → live `kcl run`; else `k8s/kustomize/overlays/<env>`; else no k8s stanza |
| namespace for live KCL | the product segment of `apps/<product>/<app>` |
| Tilt resource name | image basename (`yurikrupnik/zerg-api` → `zerg-api`), because cargo crates are `zerg_api` while the workload is `zerg-api` |
| `docker_build(only=…)` | the butler project graph — cargo dep kinds for a service, the npm workspace edges *plus* the addon's cargo closure for a node app (see below) |
| web `deps=` | whichever of `src`, `project.json`, `vite.config.ts`, `index.html` exist |
| `port_forwards` | `[tilt] hostPort` composed with the merged workload `port` — the old `portForwards = "5253:8080"` spelled the container port a second time and is gone |

## The inferred `container` / `scan` targets

Same trick, one module over. `tools/nx/container-targets.ts` merges a `container`
and a `scan` target onto every **deployable app**: an app directory with a
recognizable kind (`Cargo.toml` → service, `vite.config.ts` → web,
`astro.config.mjs` → node) that either declares a `[workload]` in its own
`butler.toml` or is named in the root `butler.toml`:

```toml
[container]
# Apps that ship an image but no workload of their own: nothing in this
# repo deploys them, but CI still builds and scans the image.
extra = ["apps/zerg/vector"]
```

That is 12 apps — the 11 Tilt apps plus `apps/zerg/vector`:

```
$ nx show projects --with-target scan
terran-web terran_api todo-astro-web todo-web todo_api todo_web_htmx todo_worker
zerg-web zerg_api zerg_email_nats zerg_tasks zerg_vector
```

`apps/butler/cli`, `todo/cli`, `todo/temporal` and `apps/zerg/shared` are
correctly out: no kind, or no workload and not listed in `extra`.

The image name is the app directory below `apps/` with `/` replaced by `-`
(`apps/zerg/email-nats` → `zerg-email-nats`) — the same rule as
`derived_image_name` in `tilt.rs`. `file` and the build-arg key come from
`[imageDefaults.<service|web|node>]`; an app's own `butler.toml` `[image]`
overrides all of it. Canonical shape, for a service — a web app additionally
gets `"dependsOn": ["build"]`, the web Dockerfile, and `DIST_PATH=<dir>/dist`,
while a node app gets `file: manifests/dockers/node.Dockerfile`,
`build-args: ["APP_DIR=apps/todo/web-astro"]`, `target: "runtime"` and **no**
`dependsOn`, because that image is self-contained — it installs the workspace
and runs the app's build inside the builder stage, where a static web app's
image only copies in the `dist/` its local `build` target produced:

```json
{
  "executor": "@nx-tools/nx-container:build",
  "options": {
    "file": "manifests/dockers/rust.Dockerfile",
    "context": ".",
    "build-args": ["APP_NAME=<cargo package>"],
    "target": "rust",
    "tags": ["$REGISTRY/<N>:latest"],
    "push": false
  },
  "configurations": {
    "ci": {
      "push": true,
      "cache-from": ["type=registry,ref=$BUILDCACHE/<N>"],
      "cache-to": ["type=registry,ref=$BUILDCACHE/<N>,mode=max,image-manifest=true,oci-mediatypes=true"],
      "metadata": {
        "images": ["$REGISTRY/<N>"],
        "tags": ["type=sha", "type=ref,event=branch", "type=ref,event=pr", "type=raw,value=$APP_VERSION,enable=$ENABLE_VERSION"]
      }
    }
  }
}
```

```json
{
  "executor": "nx:run-commands",
  "dependsOn": ["container"],
  "options": {
    "command": "trivy image --cache-backend memory $REGISTRY/<N>:latest --severity CRITICAL,HIGH --exit-code 0",
    "cwd": "{workspaceRoot}"
  },
  "configurations": {
    "ci": { "command": "trivy image --cache-backend memory $REGISTRY/<N>:sha-$(echo $SHORT_SHA | cut -c1-7) --severity CRITICAL,HIGH --format sarif --output trivy-<N>.sarif" }
  }
}
```

One shape for every app also erases two drifts the hand-written copies had
accumulated: **every** app now gets the buildx registry cache (`cache-from` /
`cache-to`, previously only `zerg_api`, `zerg_email_nats`, `zerg_tasks`,
`zerg_vector`) and **every** scan runs `trivy --cache-backend memory` (the
`terran_*` apps lacked it).

The `target` (build stage) IS emitted, and that fixed a live defect. The
hand-written targets pinned no stage, so buildx built each Dockerfile's last
one: `rust` for `rust.Dockerfile` (harmless — the stage Tilt pins anyway) but
`static-web-server` for `Dockerfile`, whose own header calls it "static-only (NO
/api proxy)". Every web Deployment (`zerg-web`, `todo-web`, `terran-web`) injects
`UPSTREAM_HOST`/`UPSTREAM_PORT`/`UPSTREAM_NAME` from a `*-proxy-envs` ConfigMap
that only the `nginx` stage consumes, and `zerg-web`'s prod HTTPRoute sends `/`
— including `/api` — straight to the pod. So CI images served the SPA and
dropped the API proxy, while Tilt images worked. The stage now resolves once
(app `[image] target` → `[imageDefaults.<kind>].target` → `[dockerfileTarget]`)
for both readers, which is why those two tables sit at the ROOT of `butler.toml`
rather than under `[tilt]`: a name that says "tilt" is what hid the bug.

Two implementations of one truth — the plugin resolves these facts from the root
`butler.toml` in TypeScript, butler resolves them again in Rust to render
Tiltfiles — so there is a gate:

```bash
just container-check     # part of `just verify`
# nx graph --file=$g  |  butler container verify --graph $g
```

butler recomputes the expected `container`/`scan` facts and diffs them against
the graph dump. A divergence fails the gate instead of shipping an image built
two different ways depending on who asked.

## The k8s manifest track

Third track, same division. nx owns the LIST (`tools/nx/k8s-targets.ts`), butler
owns the LOGIC, and a published KCL package — `oci://docker.io/yurikrupnik/app`,
pinned at `tag = "0.1.2"` in the root `[k8s]` — owns the Kubernetes shape. This
is the KCL half of the split argued for above: structured data in, objects out,
no filesystem and no dependency graph.

Which file owns which fact:

| fact | owner |
|---|---|
| `registry`, `env`, `[k8s]` (`package`, `tag`, `[k8s.imageTag]`, `extraResources`), `[imageDefaults.<kind>]`, `[dockerfileTarget]`, `[workloadDefaults.<kind>]`, `[container] extra`, `[tilt]` infra | root `butler.toml` |
| `[workload]`, optional `[config]` / `[externalSecret]`, per-env `[env.<name>.workload]`, and the dev-loop facts under `[tilt]` (`hostPort`, `resourceDeps`, `labels`) | `<app>/butler.toml` — only what DIFFERS |
| `image`, `name`, `namespace`, `partOf` | neither: butler injects them |

`[workload]` is a **pass-through payload**. butler serialises the TOML table to
YAML 1:1 and models none of the package's schema, so KCL's typed `Release`
schema is the thing that validates it — and publishing a new package version
needs no butler change at all. Merge order, deep and right-wins:

```
[workloadDefaults.<kind>]  <  <app> [workload]  <  [env.<env>.workload]
```

Identical semantics to the package's own `lib.mergeValues`, so the values butler
writes and the values KCL merges cannot disagree about precedence.

What butler injects is what an app must never hand-type. `image` is injected and
**declaring it anywhere is a hard error**, naming the offending file and table;
`name`, `namespace` and `partOf` are injected as defaults an app may override.
The reason is a defect this killed: the same image fact used to be spelled three
ways — `yurikrupnik/todo-api:dev` hardcoded in one manifest,
`yurikrupnik/zerg-api:main` in another (with commented-out GAR references beside
them), and `$REGISTRY/<name>:latest` derived by nx and butler. One home, one
spelling, or they drift; the per-env tag comes from `[k8s.imageTag]`.

## The generated k8s artifacts and their gate

```bash
butler k8s gen [--app DIR] [--root] [--check] [--apps CSV]
```

| artifact | content |
|---|---|
| `<app>/k8s/values.yaml` | the merged payload the package validates |
| `<app>/k8s/values.<env>.yaml`, one per env | that env's deltas only |
| `manifests/k8s/apps/<app>.yaml` | stdout of `kcl run "<package>?tag=<tag>" -D env=<env> -q`, run in a scratch directory holding nothing but the freshly computed values |
| `manifests/k8s/apps/kustomization.yaml` | one entry per app, sorted, plus the `[k8s] extraResources` |

All four are committed and drift-gated: `just k8s-check` ("11 k8s artifacts up
to date") sits in `just verify` beside `tilt-check` and `container-check`. The
aggregate is written from the graph, exactly as the root Tiltfile is:

```bash
butler k8s gen --root --apps "$(just _k8s-apps)"   # roots of nodes having k8s-gen
```

There is a `values.<env>.yaml` for **every** env, not only for the envs an app
declares an `[env.<name>]` table for: the package hard-asserts on a missing
overlay when `-D env=` is passed, so an absent `values.prod.yaml` is a render
failure rather than a silent no-op. dev and prod files are therefore mandatory,
and an app with nothing env-specific still gets one, carrying just that env's
image tag.

The dev loop reads the same two files: a generated `Tiltfile`'s k8s stanza is now
`k8s_yaml(local('kcl run "<package>?tag=<tag>" -D env=dev -q', dir='k8s'))`, so
Tilt renders the published package live against `<app>/k8s/values*.yaml` instead
of building a kustomize overlay. `port_forwards` is composed from `[tilt]
hostPort` and the merged workload `port`, which is why `hostPort` is a plain
number now.

## Why `[workload]` is the deployability predicate

"Ships k8s manifests" used to be the test — `k8s/kcl.mod` or
`k8s/kustomize/overlays/<env>/kustomization.yaml` on disk. It cannot be, now that
those manifests are butler's own output: keying inference on the artifact the
inference produces is circular, and a `k8s-gen` target would only appear after a
first successful `k8s-gen`. The test is now **the app declares a `[workload]`**,
in all three readers — butler's `deployable_dirs`,
`tools/nx/butler-config.ts`'s `hasWorkload`, and the Tilt app set.

The container/scan set keeps its extra clause, `kind AND ([workload] OR
[container] extra)`, which is what keeps `apps/zerg/vector` building and scanning
an image that nothing in this repo deploys.

## Dev-only fixtures: the one escape hatch

Three zerg overlays used to carry `secretGenerator` literals for dev-only
Secrets. Those are environment fixtures, not workload shape — and the package
renders **ExternalSecrets, never literal Secrets** — so they live in exactly one
place now, `manifests/k8s/dev/app-secrets.yaml`, pulled in by:

```toml
[k8s]
extraResources = ["../dev"]
```

That directory carries a `kustomization.yaml` of its own because kustomize
refuses to accumulate a file outside its root but accepts a sibling directory.

## Two intentional behaviour changes

Both fell out of removing duplication, and both are decisions with evidence, not
drift:

| decision | evidence |
|---|---|
| service `runAsUser`/`runAsGroup` is **65532** | that is the rust image's actual `USER`. `rust.Dockerfile`'s runtime stage carried three stacked `USER` lines of which only the last applies, so the 65534 the old manifests pinned matched a line that never took effect. |
| `prometheus` defaults to **false** for the service and node kinds | no app serves `/metrics` except the three that now set `prometheus = true` explicitly. The old default would have annotated four apps whose `/metrics` is a 404 — a scrape target pointing at nothing. |

## Monorepo and standalone are the same code path

Without nx there is no plugin, so butler falls back to its own identical
discovery (`butler tilt gen` with no `--apps`): a standalone repo is a monorepo
with one app at `.`, and the root file grows an `[app]` section and the app's
stanzas are inlined into the root Tiltfile instead of being `include()`d. No per-language presets, no second generator. Verified end
to end on a scratch repo (`/tmp/solo`: `Cargo.toml`, `src/main.rs`, `Dockerfile`,
`k8s/kustomize/overlays/dev`, and this config):

```toml
registry = "ghcr.io/acme"

[dockerfileTarget]
"Dockerfile" = "runtime"

[[tilt.portForward]]
name = "postgres"
command = "kubectl port-forward -n dbs deployment/postgres 5432:5432"
probePort = 5432

[app.tilt]
hostPort = 8080

[app.image]
dockerfile = "Dockerfile"
tag = "$REGISTRY/solo-svc:latest"
buildArgs = { APP_NAME = "solo_svc" }
```

→ one `Tiltfile`, with the postgres forward, `docker_build('ghcr.io/acme/solo-svc',
… only=['Cargo.toml', 'Dockerfile', 'src'], target='runtime')`,
`k8s_yaml(kustomize('k8s/kustomize/overlays/dev'))` and
`k8s_resource('solo-svc', port_forwards='8080:8080', labels=['backend'])`.

The workspace root is found by walking up for `butler.toml` (then `nx.json`), so
the CLI needs no nx in the repo it runs in.

## Build context derivation

For a service, `only=` has three groups:

1. workspace plumbing — `Cargo.toml`, `Cargo.lock`, the Dockerfile, plus any
   `dockerfileOnly` paths, filtered to those that exist;
2. every **non-dependency** workspace member's `Cargo.toml` **plus the single
   entry file cargo needs to resolve a target for it**;
3. this crate and its transitive **non-dev** dependencies, whole directories.

Group 2 is not optional. Pruning a member to its manifest alone breaks the whole
workspace:

```
error: failed to load manifest for workspace member `/…/apps/terran/api`
  no targets specified in the manifest
  either src/lib.rs, src/main.rs, a [lib] section, or [[bin]] section must be present
```

Only 11 of 45 members declare an explicit `[[bin]]`/`[lib]`; the rest rely on
autodiscovery. `member_loadable_paths` parses each manifest and adds the declared
or conventional entry file.

For a node app, `only=` is derived too, but from **two** graphs: the image
installs the bun workspace and runs `astro build` itself, so its inputs span the
JS workspace *and* the cargo graph behind the N-API addon. `discovery.rs`'s
`discover_node` wires the npm workspace edges onto `Project::deps` (not
`build_deps` — those are cargo dep kinds), and `node_only` BFSes them into four
groups:

1. this app plus its transitive workspace npm deps, whole directories — for
   `todo-astro-web`: `apps/todo/web-astro`, `libs/domains/todo`,
   `libs/native/field-selector`;
2. workspace plumbing — the root `package.json` and `bun.lock`, the Dockerfile,
   plus `Cargo.toml`/`Cargo.lock` when one of those deps is cargo-built;
3. every **other** workspace member's `package.json`: `bun install
   --frozen-lockfile` resolves the whole workspace, so a member missing its
   manifest fails the install even though this app never imports it — the npm
   mirror of the service rule's group 2;
4. the addon's own cargo build closure (`libs/core/field-selector`) plus the
   manifest + entry-file stubs for every other cargo member, exactly as in the
   service rule.

Nothing there is hand-written: a new import edge in the app moves the context by
itself. `[tilt] contextIgnore` (formerly `rustIgnore`, renamed once the node
context started sharing it — `target/`, `**/*.md`, `**/dist/`,
`apps/**/node_modules/`, …) subtracts the churn that lives *inside* those
whitelisted directories.

Measured for the service rule against the blanket `only=['apps/', 'libs/']` a
human would write:

| | context bytes | files | crates whose source edit rebuilds the image |
|---|---|---|---|
| blanket | 2.99 MB | 530 | 36 |
| derived, `zerg-api` | 1.72 MB | 337 | 20 |
| derived, `terran-api` | 0.95 MB | 220 | 13 |
| derived, `zerg-tasks` | — | — | 14 |
| derived, `zerg-email-nats` | — | — | 4 |

Proven sufficient, not assumed — a tree containing *only* the generated paths:

```
$ cargo metadata --offline                      → 0
$ cargo check --offline --locked -p zerg_api    → Finished in 33.43s   (79 paths, 2.5 MB)
$ cargo check --offline --locked -p terran_api  → Finished in 1.40s    (85 paths)
```

This is the integration [tilt-dev/tilt#6502](https://github.com/tilt-dev/tilt/issues/6502)
asks for; no Nx plugin provides it.

## The node (Astro SSR) image

`manifests/dockers/node.Dockerfile` is parameterised by `APP_DIR` the way
`rust.Dockerfile` is by `APP_NAME`, and has two stages:

1. **builder** — `rust:1-slim-trixie` plus bun (installed from bun.sh at a
   pinned version): `bun install --frozen-lockfile`, then `bun run build` in
   `libs/native/field-selector` to compile the N-API addon, then `bun run build`
   in `$APP_DIR`. The addon is built *here* because it is a platform binary: a
   developer's `field-selector.darwin-arm64.node` cannot load in a linux image,
   and `.dockerignore` carries `**/*.node` so it cannot even reach the context.
2. **runtime** — `node:26-trixie-slim`, the stage `[imageDefaults.node].target`
   pins. The same Debian release as the builder on purpose: the addon is a glibc
   cdylib, so the two libcs must match.

The runtime copies only `dist/`, the app's `package.json` and a pruned
`node_modules`. Pruning is not `bun install --production`: bun 1.4's isolated
linker makes the root `node_modules` a 723 MB store of symlinks into
`node_modules/.bun`, and a later `--production` install against it is a no-op.
So the builder runs a small script that walks the built `dist` for bare
specifiers, resolves each through node's real module search and materialises
only that closure — for `web-astro` (`@native/field-selector` + `solid-js`)
5.0 MB instead of 723 MB.

The runtime contract matches the sibling images so k8s manifests can treat every
app alike: `USER 1000:1000`, `NODE_ENV=production HOST=0.0.0.0 PORT=8080`,
`CMD ["node", "dist/server/entry.mjs"]`, and `GET /health` → 200 for the probes
(`apps/todo/web-astro/src/pages/health.ts`; `todo/web-htmx` already served
`/healthz`, which is what its own manifests probe).

## No Rust `live_update`

`manifests/dockers/rust.Dockerfile`'s `rust` stage is `FROM scratch`: no shell,
no cargo. `sync('src', '/app/…')` would have Tilt report a successful live update
while the running binary stayed stale. A source change rebuilds the image; the
tight `only=` list is what makes that affordable.

## What was removed and why

| removed | reason |
|---|---|
| `manifests/tilt/tilt.toml` | superseded by the root `butler.toml`; the CLI's config belongs at the repo root next to the repo it configures |
| `metadata.tilt` in six `project.json` files | app config now lives in `<app>/butler.toml`, so the CLI does not require nx-shaped files |
| `scripts/kcl/tilt/**` (KCL generator, ~800 lines) | it was a second process with a second config place: `main.k` hand-transcribed the registry, the three infra port-forwards, the shared ConfigMap, and every app's crate name / port / image / stage — all of which the root and app `butler.toml` (or project.json) already own. Its one unique feature, per-language presets for standalone repos, is now the `[app]` section above. Restore with `git checkout 3001a15 -- scripts/kcl/tilt` if ever needed. |
| `just tilt-gen-kcl`, `tilt-lint-kcl`, `tilt-gen-diff`, `s-kcl` | no second generator to drive or diff against |
| `tools/tilt/plugin.ts` (the path, not the plugin) | split into `tools/nx/tilt-targets.ts` and registered through `tools/nx/plugin.ts`, next to `rust-targets.ts`, `container-targets.ts` and `k8s-targets.ts`; the four share `tools/nx/butler-config.ts` for reading the root `butler.toml`, so one directory holds every local plugin and `nx.json` holds one entry |
| hand-written `build` / `run` / `container` / `scan` targets in 27 `project.json` files (1347 lines) | now inferred: `tools/nx/rust-targets.ts` from each `Cargo.toml`, `tools/nx/container-targets.ts` from the root `butler.toml`. project.json targets *override* inferred ones, so a leftover copy silently shadows inference — deleting them was the cutover, not tidying. `@monodon/rust` still infers the nodes and dep edges (it only ever inferred the `nx-release-publish` target), so the projects themselves are untouched. |
| every `apps/*/*/k8s/kustomize/**` tree for the 11 workload apps | the workload is a `[workload]` table rendered by the pinned KCL package; a hand-written overlay beside it is a second spelling of the same fact. `apps/zerg/shared/k8s/kustomize` STAYS — it has no app kind and the root `[[tilt.sharedResource]]` references it by path. |
| `apps/terran/{api,web}/k8s/{main.k,kcl.mod,kcl.mod.lock}` | per-app KCL modules depending on `manifests/kcl/app`, a package path that no longer exists — `kcl run` failed with `CannotFindModule`. Both apps now render the published package like every other app. |
| `manifests/apps/core/backend` and its only consumer, `apps/terran/api/k8s/kustomize/overlays/local` | a superseded earlier attempt at the same cross-app-DRY goal, referenced nowhere else. |
| dev-only `secretGenerator` literals in three zerg overlays | environment fixtures, not workload shape; one file now, `manifests/k8s/dev/app-secrets.yaml`, reached through `[k8s] extraResources`. |

Before removal both generators were run head to head: identical output except the
header comment, the `only=` list, and web `deps=` — i.e. the KCL track's 249
hand-maintained config lines bought nothing except the blanket build context and
a missed `index.html` watch. Findings that came out of that comparison and are
worth keeping:

* the `FROM scratch` / `live_update` bug above (both generators emitted it,
  copied from the pre-existing `apps/terran/api/Tiltfile`);
* `apps/zerg/vector/project.json` declared a Tilt port-forward `50051:50051` —
  colliding with `zerg-tasks` — for an app with no k8s manifests at all; the
  derivation failed loudly, the static config silently disagreed with the tree;
* `apps/zerg/email-nats/project.json` carried an `extraResources` mailhog
  port-forward duplicating the infra one.

## Known unrelated breakage — resolved

`apps/terran/{api,web}/k8s` were KCL modules depending on package
`manifests/kcl/app`, which existed neither in this repo nor anywhere in its git
history: `kcl run` failed with `CannotFindModule`, so the generated
`k8s_yaml(local('kcl run k8s …'))` never loaded. Both modules are deleted and
both apps render `oci://docker.io/yurikrupnik/app` like every other app — which
also retires the "extending the CLI to generate k8s resources" plan that used to
close this document. It is the k8s track above, shipped: `[k8s]` in the root
file, `[workload]` in the app file, `butler k8s gen` writing `<app>/k8s/` and
`manifests/k8s/apps/`.
