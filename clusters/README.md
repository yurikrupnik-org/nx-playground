# Clusters

Multi-cluster GitOps setup: kind (local) + GKE (real), per org (paidevo, yurikrupnik).
GCP via Workload Identity (no JSON keys), AWS-ready in the schema.

## Cluster inventory

| Directory | Provider | GCP project | Org | Notes |
|---|---|---|---|---|
| `dev/paidevo-local`     | kind | `bootstrap-491220` | paidevo     | Local kind, WIF via GCS-published JWKS |
| `dev/paidevo-gke`       | gke  | `bootstrap-491220` | paidevo     | Real GKE `paidevo-cluster` in `me-west1` |
| `dev/yurikrupnik-local` | kind | `yk-admin-gke`     | yurikrupnik | Local kind |
| `dev/yurikrupnik-gke`   | gke  | `yk-admin-gke`     | yurikrupnik | Planned cluster (does not exist yet) |

## Architecture (per cluster)

```
inputs.yaml                  hand-edited source of truth
  ↓ kcl run
cluster-config.yaml          ConfigMap (used by Flux postBuild substitution)
wif/wif.yaml                 namespaces + KSAs (+ kind: gcp-wif-config Secret)
cluster/kind-config.yaml     (kind only) kind cluster config
oidc/openid-configuration    (kind only) OIDC discovery doc

flux-root.yaml               three chained Flux Kustomizations:
  1. infrastructure-controllers   → overlays/controllers-{kind,gke}
  2. infrastructure-packages      → overlays/packages       (dependsOn 1)
  3. infrastructure-configs       → overlays/configs-{kind,gke}  (dependsOn 2)
```

## Prerequisites (machine-level, one-time)

```bash
brew install just kcl-lang/kcl/kcl yq kubectl kind flux gcloud mkcert
mkcert -install      # registers mkcert's root CA in your system trust store
gcloud auth login    # authenticate as the human
```

GitHub PAT with `repo` scope exported as `GITHUB_TOKEN` (Flux bootstrap pushes flux-system manifests).

## Per-cluster pre-flight checklist

For each cluster you intend to bring up:

### 1. Confirm `inputs.yaml` is complete

```bash
yq '{wifProjectNumber, acmeEmail}' clusters/dev/<c>/inputs.yaml
```

- **kind clusters**: `wifProjectNumber` must be non-zero (look up via `gcloud projects describe <p> --format='value(projectNumber)'`)
- **gke clusters**: `acmeEmail` must be set (Let's Encrypt account registration)

### 2. Confirm GSAs exist in the target GCP project

```bash
proj=$(yq -r .gcpProject clusters/dev/<c>/inputs.yaml)
gcloud iam service-accounts list --project="$proj" \
  | grep -E "external-secrets|crossplane-controller"
```

If missing, create them and grant the roles they need:

```bash
gcloud iam service-accounts create external-secrets --project="$proj"
gcloud iam service-accounts create crossplane-controller --project="$proj"

gcloud projects add-iam-policy-binding "$proj" \
  --member="serviceAccount:external-secrets@$proj.iam.gserviceaccount.com" \
  --role="roles/secretmanager.secretAccessor"

# Grant Crossplane the roles it needs to manage GCP resources, e.g.:
gcloud projects add-iam-policy-binding "$proj" \
  --member="serviceAccount:crossplane-controller@$proj.iam.gserviceaccount.com" \
  --role="roles/editor"   # narrow this in real use
```

### 3. Re-render with current inputs

```bash
just cluster-render <c>
```

### 4. Commit + push (Flux pulls from the remote)

```bash
git add clusters infrastructure kcl justfile
git commit -m "wire <c>"
git push
```

If you don't want to merge to `main` yet, set `githubBranch: <test-branch>` in `inputs.yaml`, render, push that branch.

## Run sequence — kind cluster

```bash
# one-time per GCP project: creates GCS bucket + WIF pool + per-workload providers
just wif-bootstrap <c>

# render → kind create → oidc upload → cluster-grant → flux bootstrap
just cluster-up <c>

# watch Flux reconcile (controllers → packages → configs)
flux get kustomizations -A -w

# once stage 1 (infrastructure-controllers) is Ready,
# install the mkcert root CA so ca-local ClusterIssuer can sign
just kind-ca-install <c>

# verify
kubectl --context=kind-<clusterName> get clusterissuer ca-local
```

## Run sequence — gke cluster

```bash
# precondition: the GKE cluster exists, gcloud auth done as a user with cluster admin
just cluster-up <c>

# watch
flux get kustomizations -A -w

# verify
kubectl --context=$(kubectl config current-context) get clusterissuer letsencrypt-prod
```

No `wif-bootstrap`, no `kind-ca-install`. GKE uses native Workload Identity, and Let's Encrypt issues real certs once the gateway IP has correct DNS.

## Expected reconcile timeline

| Stage | Duration | Health gate |
|---|---|---|
| `infrastructure-controllers` | 30–90s | Deployments: cert-manager, external-secrets, crossplane, kyverno-admission-controller |
| `infrastructure-packages` | 60–180s | Crossplane `Provider/provider-gcp-{cloudplatform,storage}` Healthy |
| `infrastructure-configs` | 10–30s | ClusterIssuer, ClusterSecretStore, ProviderConfig, ClusterPolicies applied |

If a stage hangs:

```bash
flux logs --follow --level=error
flux get kustomizations -A    # see which Kustomization is stuck
kubectl get events -A --sort-by=.lastTimestamp | tail -30
```

Typical causes:
- **Stage 1 stuck**: HelmRepository unreachable, chart pull failed, namespace not created.
- **Stage 2 stuck**: Crossplane Deployment not Ready (probably failing image pull or RBAC), Provider package fails to install.
- **Stage 3 stuck**: GSA missing or `cluster-grant` not run (token exchange returns `PERMISSION_DENIED`); ACME email not set; mkcert Secret missing.

## Tear down

```bash
just cluster-down <c>            # kind: deletes cluster; gke: refuses
```

Note: `flux bootstrap` writes `clusters/dev/<c>/flux-system/` to git. Re-bootstrapping reuses it. Delete the dir manually if you want a clean slate.

## Adding a new cluster

1. `cp -r clusters/dev/<existing> clusters/dev/<new>` (pick the closest provider+org)
2. Edit `inputs.yaml`: change `org`, `oidcId`, `clusterName`, `gcpProject`, `gatewaySuffix`, GSA emails
3. `just cluster-render <new>`
4. Commit, push, then run the appropriate sequence above.

Schema reference: see `kcl/main.k` for the full set of `option(...)` keys, and `kcl/schemas.k` for `Workload`/`CloudBindings`.

## File map

```
clusters/
  dev/<cluster>/
    inputs.yaml                source of truth (hand-edited)
    cluster-config.yaml        rendered (DO NOT edit)
    wif/wif.yaml               rendered (DO NOT edit)
    cluster/kind-config.yaml   rendered, kind-only (DO NOT edit)
    oidc/openid-configuration  rendered, kind-only (DO NOT edit)
    kustomization.yaml         Kustomize entry — references the rendered files + flux-root
    flux-root.yaml             3 Flux Kustomizations (controllers/packages/configs)

infrastructure/
  addons/
    cert-manager/{controller,issuers/{kind,gke}}/
    external-secrets/{controller/{base,kind,gke},store}/
    crossplane/{controller,packages,configs/{kind,gke}}/
    kyverno/{controller,policies}/
    overlays/
      controllers-{kind,gke}/   stage 1
      packages/                 stage 2
      configs-{kind,gke}/       stage 3

kcl/
  main.k       reads inputs.yaml options, emits cluster-config / wif / kind-config / oidc
  schemas.k    Workload + CloudBindings types

justfile     all recipes (cluster-render/up/down/grant, wif-bootstrap, kind-ca-install)
```
