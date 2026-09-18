---
name: kcl-package
description: Create, version and publish a KCL package as an OCI artifact to a registry of the user's choice (Docker Hub, ghcr.io, a local kind registry), and consume it from this repo. Use when adding or bumping a KCL package, changing `[k8s] package`/`tag` in butler.toml, or when `kcl mod push`/`kcl run oci://` fails.
---

# KCL package → OCI

Packages live in the sibling repo `~/gitorgs/kcl-packages` (an nx workspace
where every `kcl.mod` IS a project via `tools/nx-kcl`). This repo only
CONSUMES them: `butler.toml` `[k8s] package = "oci://docker.io/yurikrupnik/app"`,
`tag = "0.1.2"` renders `~/gitorgs/kcl-packages/packages/app`. Never create a
KCL package inside nx-playground (`apps/terran/*/k8s/{main.k,kcl.mod}` was that
mistake — `CannotFindModule`, see `docs/tilt-generators.md`).

## 1. Create

In `~/gitorgs/kcl-packages`:

```bash
nx g nx-kcl:package <name> --directory=packages/<area> --dependencies=k8s:1.32.4
```

Writes `<name>.k` (schemas), `main.k` (`import .<name>`, values-driven entry),
`<name>_test.k` (`test_* = lambda { … }`), then `kcl mod init` + `kcl mod add`.
Rules the generator and `just mod-check` enforce:

- `[package] name` is the nx project name AND the OCI image name; must be unique
  across the workspace (a duplicate silently merges two projects). Hyphens are
  fine in the name but imports use `_` (`chaos-mesh` → `import chaos_mesh`).
- `kcl mod init` owns `edition`; never hand-write it. Adding deps: `nx run
  <name>:add k8s:1.32.4` / `--oci https://ghcr.io/kcl-lang/helloworld --tag 0.1.0`.
  Removing: `nx run <name>:remove <dep>` (there is no `kcl mod remove`).
- Layout must be `packages/<area>/…`: the `area:<segment>` tag drives release
  scoping. `packages/providers/*` (CRD-generated schemas, `nx g nx-kcl:import-crd`)
  get no targets and are NEVER published — depend on them by version-less
  `x = { path = "../providers/x" }`; a version pin breaks on the next bump. The
  publisher vendors path deps into the artifact, so the image is self-contained.
- Follow `packages/app`: `lib.k` schemas + renderers, `main.k` reads
  `option("values")`/`option("env")`, deep-merges `<stem>.<env>.yaml`, ends in
  `manifests.yaml_stream(...)`; renders a demo when no values file so `nx build`
  still smoke-tests. Docs go in the `main.k`/`lib.k` docstring and `examples/`
  (there are no package READMEs). `-D values=` is CWD-relative.
- Gate: `nx run-many -t build test lint --projects=<name>` (= `kcl run main.k`,
  `kcl test`, `kcl lint`), `just check` for everything. lefthook runs
  `fmt`/`lint`/`mod-check` on commit and `just check` on push.

## 2. Choose the registry

ONE string, `KCL_REGISTRY` (no scheme, `<host>/<namespace>`), resolved the same
way by versioning (`version-actions.ts:resolveRegistry`) and publishing
(`publish-executor.ts`): executor `registry` option → plugin `registryPrefix`
(WITH `oci://`) → `$KCL_REGISTRY` env → `nx.json` `release.registry`
(currently `docker.io/yurikrupnik`). The push target is
`oci://$KCL_REGISTRY/<name>`; the tag is `[package] version` from `kcl.mod` —
there is no `--tag`.

| target | how |
|---|---|
| repo default (Docker Hub) | nothing to set |
| ghcr.io / another namespace | `export KCL_REGISTRY=ghcr.io/<org>` for the release commands below, or change `nx.json` `release.registry` to move the repo default |
| local kind registry (e2e) | `just registry` (`registry:2` on `localhost:5001`, in-cluster `kind-registry`, pinned `172.18.0.100` on the `kind` network), then `just publish-all` / `just e2e-publish <module>` — they set `KCL_REGISTRY=localhost:5001` |

Credentials — the hard-won part (`.github/workflows/release.yml:105-134`):
`kcl mod push oci://docker.io/...` looks the credential up under the LITERAL host
key `docker.io`. `docker login` stores Docker Hub under
`https://index.docker.io/v1/` (lookup misses → 401), and `kcl registry login
docker.io` fails to persist for https registries ("putting plaintext credentials
is disabled"). Write kpm's own file, no `credsStore`:

```bash
export KCL_PKG_PATH="$HOME/.kcl/kpm"          # pin it: kpm's default home moved in 0.12.5
mkdir -p "$KCL_PKG_PATH/.kpm/config"
printf '{"auths":{"%s":{"auth":"%s"}}}' "<host>" "$(printf '%s:%s' "$USER" "$TOKEN" | base64)" \
  > "$KCL_PKG_PATH/.kpm/config/config.json"
```

Local secrets come from vals via devkit (`devkit.toml` `[secrets]`: `.vals.yaml`
→ `.env` with `DOCKERHUB_USERNAME`/`DOCKERHUB_TOKEN`); never paste tokens into
files. Plain-HTTP
registries need `OCI_REG_PLAIN_HTTP=on` (exactly `on`/`off`; `true` hard-errors
and is sticky), which breaks ghcr.io's token exchange — that is why the local
flow mirrors `k8s` first (`just registry-seed-k8s`). An OCI ref whose first
segment has no dot/port is a Docker Hub namespace (`kind-registry/pkg` →
`index.docker.io/kind-registry/pkg`).

## 3. Version & publish

Versions are Conventional-Commit driven by `nx release` (git tag
`<name>@<version>`, per-project `CHANGELOG.md`, `version` rewritten in `kcl.mod`,
any sibling `composition.yaml` `source:` re-pinned to
`oci://<registry>/<name>?tag=<version>`). From `~/gitorgs/kcl-packages`:

```bash
just release-dry                 # preview: no commit, tag or push
just release                     # version + changelog + tag + kcl mod push
just release-first 0.1.0         # very first release (no tags yet)
just release-publish             # retry the OCI push only (tags already landed)
```

Order is git first, OCI second: a published version can never be re-pushed
(`kcl mod push` refuses; the executor treats "already exists" as idempotent
success on retry, `--force` is only for the throwaway local mirror). CI
(`release.yml`, main only, `concurrency: release`) does the same on push to main;
`ci.yml` gates PRs with `nx affected -t build test lint`, and `nx-release-publish`
`dependsOn: [test, lint]`, so a broken package cannot publish.

## 4. Consume from nx-playground

- Workload rendering: bump `butler.toml` `[k8s] tag` to the released version
  (check `~/gitorgs/kcl-packages/packages/app/CHANGELOG.md`; keep it PINNED —
  an unpinned ref re-renders on every publish), then `just k8s-gen`,
  `just tilt-gen`, and commit the regenerated `manifests/k8s/apps/**`,
  `<app>/k8s/values*.yaml` and `Tiltfile`s (`just k8s-check` / `just tilt-check`
  are the drift gates). `[k8s] package` is a bare `oci://<host>/<ns>/<name>`;
  the tag lives only in `tag`.
- Ad hoc: `kcl run "oci://docker.io/yurikrupnik/app?tag=0.1.2" -D env=dev -q`
  (what the generated Tiltfiles run).
- As a dependency of another KCL module:
  `app = { oci = "oci://docker.io/yurikrupnik/app", tag = "0.1.2" }` in
  `[dependencies]`; local dev alternative `app = { path = "…/packages/app" }`.
- Crossplane: `KCLInput.spec.source: oci://<registry>/<name>?tag=<version>` —
  the tag is rewritten by `nx release`; `just mod-check` fails if the image name
  does not match the package's own name. In-cluster pulls from a private/local
  registry need an RFC1918 IP with a dot (`172.18.0.100`), not `localhost:5001`.
