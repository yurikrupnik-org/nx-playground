# Local-cluster dev environment + compose → manifest CLI

## Goal

Spin up a local kind cluster with a curated set of platform services (nats, cnpg, redis, external-secrets, qdrant, influxdb, flagsmith, keycloak, testkube, mailhog) using a single declarative source: a `compose.yaml`-style file with labels that decide whether each service is rendered as a Helm-based resource or a kompose-converted raw Deployment. The CLI emits to one of three GitOps targets (Flux, Argo, Crossplane) or to plain YAML.

## What already exists (do not reinvent)

`~/dotconfig` has the bones of this CLI. The compose-driven generator pattern is half-built and mature in places:

| Component | Path | Status | Reuse |
|-----------|------|--------|-------|
| Compose parser | `~/dotconfig/src/utils/compose_parser.rs` (591 lines) | Mature — handles ports/labels/depends_on/env in all compose-spec variants | Use as-is |
| KCL bridge | `~/dotconfig/src/utils/kcl_bridge.rs` | Working — shells out to `kcl` CLI with JSON spec, returns rendered YAML | Extend with per-target render methods |
| Compose action (up/down/kompose-convert) | `~/dotconfig/src/actions/compose.rs` (352 lines) | Working | Reuse for `up/down`; the convert path is the kompose anchor |
| **Flux generator** | `~/dotconfig/src/actions/flux.rs` (218 lines) | Working — `FluxAction::Generate` reads compose + renders via KCL | **This is the template.** Generalize, don't fork |
| Helm client | `~/dotconfig/src/operator/helm/client.rs` (298 lines) | Used by the operator path; not needed for *generation* | Skip for now |
| KCL modules | `nx-playground/kcl/{main.k, schemas.k}` | Existing schemas | Add per-target renderers next to these |

The shape that already exists in `flux.rs`: parse compose → build a `PlatformSpec` JSON → `KclBridge::render_flux(spec_json)` → write YAML. Generalizing this to `render(target, spec_json)` is the central refactor.

## Design

### One CLI command, three target backends

Replace `FluxAction` with a target-agnostic action. `~/dotconfig/src/actions/`:

```rust
// new: src/actions/cluster.rs (or rename flux.rs → cluster.rs)
pub enum ClusterAction {
    /// Generate cluster manifests from a compose file
    Generate(GenerateArgs),
}

pub struct GenerateArgs {
    file: Option<String>,                // compose path
    namespace: String,
    target: Target,                      // flux | argo | crossplane | raw
    output: Option<PathBuf>,             // - for stdout, dir for split, file for single
    repo_url: Option<String>,            // flux/argo only
    repo_branch: String,
    interval: String,
    kcl_dir: Option<PathBuf>,
}

pub enum Target { Flux, Argo, Crossplane, Raw }
```

`KclBridge` gains:

```rust
impl KclBridge {
    pub fn render(&self, target: Target, spec_json: &str) -> Result<String> {
        let renderer = match target {
            Target::Flux       => "renderer/from_compose_flux.k",
            Target::Argo       => "renderer/from_compose_argo.k",
            Target::Crossplane => "renderer/from_compose_crossplane.k",
            Target::Raw        => "renderer/from_compose_raw.k",
        };
        run_kcl_with_arg(&self.kcl_dir, Path::new(renderer), &format!("spec={spec_json}"))
    }
}
```

The existing `render_flux` becomes a thin wrapper for backwards compat (or just gets deleted).

### Label convention on the compose file

Each service in `compose.yaml` declares its packaging via labels. This is the only new "DSL" — everything else is standard compose.

```yaml
services:
  nats:
    labels:
      dotconfig.kind: helm
      dotconfig.helm.chart: oci://registry-1.docker.io/bitnamicharts/nats
      dotconfig.helm.version: "8.x"
      dotconfig.helm.values: |
        cluster:
          enabled: false
        jetstream:
          enabled: true

  mailhog:
    image: mailhog/mailhog:latest
    ports: ["1025:1025", "8025:8025"]
    # no dotconfig.kind label → falls through to kompose-style raw Deployment
```

Resolution rule:
- `dotconfig.kind: helm` → emit `HelmRelease` (flux) / `Application` w/ helm source (argo) / `Release.helm.crossplane.io` (crossplane) / a rendered `helm template` snippet (raw).
- `dotconfig.kind: manifest` → user-supplied raw manifest path via `dotconfig.manifest.path`.
- *no label* → kompose-convert the service (Deployment + Service from image+ports+env).

Add to the compose parser: a `dotconfig` substruct extracted from the labels map. Keep labels untouched on the wire so docker-compose itself still works.

### Output sink

Single `--output` flag, three behaviors driven by what the value points to:

| Value | Behavior |
|-------|----------|
| omitted or `-` | stdout |
| `path/to/file.yaml` | single multi-doc YAML |
| `path/to/dir/` (trailing slash or existing dir) | one file per resource: `<kind>-<name>.yaml` |

This matches `pg-cli`'s existing pattern in `~/dotconfig/crates/pg-cli/src/main.rs:write_or_print` — lift that helper into `~/dotconfig/src/utils/output.rs` so both CLIs share it.

## Service inventory for local dev

Using OCI/community charts where they exist, kompose-raw otherwise. Pin versions explicitly.

| Service | Strategy | Chart / Image | Notes |
|---------|----------|---------------|-------|
| **nats** | helm | `oci://registry-1.docker.io/bitnamicharts/nats` | jetstream on, single replica |
| **cnpg** | helm (operator) + manifest (cluster) | operator: `oci://ghcr.io/cloudnative-pg/charts/cloudnative-pg`. Cluster: reuse `manifests/db/zerg/k8s/overlays/dev/` | Operator chart + existing zerg overlay |
| **redis** | helm | `oci://registry-1.docker.io/bitnamicharts/redis` | architecture=standalone, auth disabled |
| **external-secrets** | helm | `oci://ghcr.io/external-secrets/charts/external-secrets` | CRDs included |
| **qdrant** | helm | `https://qdrant.to/helm` chart `qdrant` | persistence=false for local |
| **influxdb** | helm | `oci://registry-1.docker.io/bitnamicharts/influxdb` | v2, single node |
| **flagsmith** | helm | `https://flagsmith.github.io/flagsmith-charts` chart `flagsmith` | needs Postgres — point at cnpg or its bundled subchart |
| **keycloak** | helm | `oci://registry-1.docker.io/bitnamicharts/keycloak` | dev mode, in-memory or cnpg-backed |
| **testkube** | helm | `https://kubeshop.github.io/helm-charts` chart `testkube` | already partially set up — see `chore(testkube)` in recent commits |
| **mailhog** | kompose-raw | `mailhog/mailhog:latest` | tiny, no chart needed |

Compose file lives at `nx-playground/manifests/local/compose.yaml` (new dir — keeps it separate from the existing `manifests/dockers/compose.yaml` which is for a different purpose).

## Steps

### 1. Refactor `flux.rs` → `cluster.rs` in `~/dotconfig`
- Extract `Target` enum, `GenerateArgs` struct.
- Move target-selection logic out of the action and into `KclBridge::render`.
- Keep `FluxAction` as a deprecated alias that calls `ClusterAction::Generate { target: Flux, .. }` until callers migrate.

### 2. Add label parsing to `compose_parser.rs`
- New struct `DotconfigLabels { kind: Option<String>, helm: Option<HelmLabels>, manifest: Option<ManifestLabels> }`.
- Extract from existing `labels: BTreeMap<String, String>` after deserialization. Keep raw labels intact.
- Unit test: feed a sample compose with mixed services, assert each parses to the right enum variant.

### 3. Author KCL renderers in `nx-playground/kcl/renderer/`
Four files, each takes the same input spec shape:

```
renderer/
  from_compose_flux.k         # HelmRelease + HelmRepository / OCIRepository
  from_compose_argo.k         # Application (helm source) / Application (directory source)
  from_compose_crossplane.k   # Release.helm.crossplane.io + Object.kubernetes.crossplane.io
  from_compose_raw.k          # Plain Deployment + Service (kompose-equivalent)
```

KCL is the right place because the structural diff between targets is mostly YAML shape, and the schemas in `kcl/schemas.k` already cover the platform primitives.

### 4. Author `manifests/local/compose.yaml`
Codify the inventory above. One service per platform component, labels chosen per the resolution rule.

### 5. Wire into Tiltfile
Two options:

(a) **Generated-at-tilt-up**: a `local_resource('platform-manifests', cmd='dotconfig cluster generate --target flux --output manifests/local/generated/')` that re-runs on compose-file changes. Then `k8s_yaml(kustomize('./manifests/local/generated'))`.

(b) **Pre-generated, committed**: run the CLI manually, commit `manifests/local/generated/`. Tilt just loads it.

(a) is more dev-loop friendly. (b) is more reproducible/auditable. Default to (a) for local-only paths; prefer (b) for anything that ends up in another environment.

### 6. CRD ordering
External-secrets, cnpg, testkube ship CRDs. Kustomize doesn't natively wait for CRDs to register before applying CRs. Two mitigations:
- Flux `dependsOn` on the HelmReleases — supported via `dotconfig.helm.dependsOn` label.
- For Argo: `argocd.argoproj.io/sync-wave` annotation, also exposable via a label.
- For raw (Tiltfile): `resource_deps=` chain in the local_resources.

The renderer should emit these from compose's existing `depends_on:` list. Don't invent a new key.

## Crossplane: should it actually be a target?

Worth a sanity check. Crossplane shines for managing **cloud resources** (RDS, S3, GCS) via XRDs/Compositions. Using it to deploy in-cluster apps is supported (`provider-helm`'s `Release`, `provider-kubernetes`'s `Object`) but unusual — most teams do that via Flux/Argo and reserve Crossplane for cloud APIs.

**Recommendation:** ship Flux + Argo first. Add Crossplane as a stub that emits `Release` CRs but call out in docs that it's primarily for *cloud-resource provisioning* compositions (which the operator code in `~/dotconfig/src/operator/controllers/` already does for Postgres/Redis), not the right tool for "deploy nats in my cluster."

If you push back and *do* want Crossplane as a primary target, then the right pattern is:
- Crossplane provider-helm + provider-kubernetes installed by Flux first (chicken/egg bootstrap).
- Compose services emit `Release` CRs.
- This is more layers, not less.

I'd skip it for v1.

## Risks / gotchas

- **Bootstrap order on a fresh kind cluster.** Flux/Argo/Crossplane controllers themselves need installing before any of the generated CRs exist. Solution: a small bootstrap target in the CLI (`dotconfig cluster bootstrap --target flux`) that does `flux install` / `argocd install` first. Out of scope for v1 if Tilt + a one-shot script suffice.
- **Helm chart version drift.** Pin every chart with a fixed version in the compose labels. Renovate-like flow can keep them current, but no `latest`.
- **KCL schema sprawl.** Four renderers means four KCL files that mostly shape the same data. Factor common helpers into `kcl/renderer/_helpers.k` early or it'll fork.
- **Compose label namespace.** Use `dotconfig.*` consistently. Do *not* mix in `io.tilt.*`, `kompose.*`, etc., in the same file or the parsing precedence gets murky.
- **`@monodon/rust` & nx-playground crates ≠ `~/dotconfig` crates.** The CLI lives in `~/dotconfig`, distributed via `cargo install --path ~/dotconfig` (same convention as `pg-cli`). Don't try to add it to the nx-playground Cargo workspace — that'd entangle two repos.
- **Existing partial state in this repo.** `manifests/cnpg/` and `manifests/k8s/base/cnpg/` overlap (flagged in the schema cleanup). Pick one canonical source for the cnpg cluster definition before this work consumes it.

## Done when

- `dotconfig cluster generate --file manifests/local/compose.yaml --target flux --output manifests/local/generated/` produces a kustomize-loadable directory.
- The same command with `--target argo` and `--target raw` produces working equivalents.
- `tilt up` from a fresh kind cluster lands all 10 services healthy within ~5 minutes.
- A new platform service is added by editing `compose.yaml` only — no Rust/KCL changes needed for the common case (helm chart with values).

## Rough effort

- Refactor flux.rs → cluster.rs + label parsing: **0.5–1 day**.
- KCL renderers (flux exists; argo + raw new; crossplane stub): **1–2 days**.
- Compose authoring + chart pinning + verification on kind: **1 day**.
- Tiltfile + bootstrap script: **0.5 day**.
- **Total: 3–5 days** for v1 across both repos.
