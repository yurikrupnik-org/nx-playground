# Platform: DevEnvironment manager

A Crossplane-based dev-environment manager. One claim provisions an isolated
namespace with **postgres (CNPG) + redis + NATS/JetStream** on the local kind
cluster; SDKs in **rust, python, and node** create and consume environments.

```
DevEnvironment claim ──► XDevEnvironment ──► function-kcl pipeline
                                              ├─ Namespace            devenv-<name>
                                              ├─ CNPG Cluster         <name>-pg   (operator CR)
                                              ├─ redis Deploy+Svc     (raw Objects)
                                              └─ NATS helm Release    (official chart)
```

## Usage

```bash
just platform-install          # one-time per cluster: CNPG + providers + XRD + composition
just env-create myenv          # or: kubectl apply -f platform/dev-env/examples/demo.yaml
just env-status myenv
just env-delete myenv          # tears down everything, namespace included
```

Claim spec (all optional, defaults on):

```yaml
spec:
  parameters:
    postgres: { enabled: true, instances: 1, storageGB: 1 }
    redis:    { enabled: true }
    nats:     { enabled: true, jetstream: true }
```

Status contract (what SDKs consume):

```yaml
status:
  environment:
    namespace: devenv-<name>
    postgresSecret: <name>-pg-app     # CNPG secret in that namespace (uri/username/password/host/port/dbname)
    redisHost: redis.devenv-<name>.svc.cluster.local:6379
    natsUrl: nats://nats.devenv-<name>.svc.cluster.local:4222
```

## SDKs

Same surface everywhere: `create / get / connection / delete`. All three accept the
full claim surface: postgres `instances` (1-3) and `storageGB` (1-20), plus NATS
`jetstream`. Parameters left unset are omitted from the claim so the XRD defaults
apply — Rust takes a `DevEnvSpec`, python keyword arguments
(`instances=`/`storage_gb=`/`jetstream=`), node a trailing options object
(`{ instances, storageGB, jetstream }`).

| Language | Location | Run the example |
|---|---|---|
| Rust | `libs/platform/devenv-sdk` (workspace member) | `cargo run -p devenv-sdk --example status -- demo` |
| Python | `platform/sdk/python` | `uv run platform/sdk/python/example.py demo` |
| Node | `platform/sdk/node` | `node example.mjs demo` (after `npm install` in that dir) |

**Node caveat:** run examples with `node`, not `bun` — `@kubernetes/client-node`
supplies the cluster CA and client certs through a custom https agent, which
bun's fetch ignores (TLS verification + client auth both fail). `devenv.mjs`
throws immediately under bun instead of failing with an opaque TLS error, and the
package is deliberately kept out of the root bun workspace so bun never installs
or runs it.

## CloudInventory (observe-only)

A second XRD in `platform/cloud-inventory/` inverts the direction: instead of
provisioning, it **observes** resources that already exist and publishes them as
a read-only inventory. Every composed resource is created with
`managementPolicies: ["Observe"]`, so Crossplane never creates, patches, adopts
or deletes the target — deleting the claim leaves it untouched.

```
CloudInventory claim ──► XCloudInventory ──► function-kcl ──► Object (Observe) per target
                                                              └─ status.atProvider.manifest = live resource
```

```bash
just inventory-create            # applies platform/cloud-inventory/examples/demo.yaml
just inventory-status demo
just inventory-delete demo       # observed resources survive
```

Claim spec — `targets` is required; `cluster` selects an in-cluster k8s object.
The schema is provider-agnostic on purpose: a GCP composition adds a sibling
key (e.g. `gcp:`) to the same `v1alpha1` version.

```yaml
spec:
  parameters:
    targets:
      - name: coredns                 # composition-resource-name, DNS-label safe
        resourceType: compute         # compute|storage|database|network|serverless|analytics|other
        cluster:
          apiVersion: apps/v1
          kind: Deployment
          name: coredns
          namespace: kube-system      # omit for cluster-scoped resources
```

`resourceType` values are exactly the serde representation of `ResourceType` in
`libs/domains/cloud_resources` — the API maps them 1:1.

Status contract:

```yaml
status:
  inventory:
    observed: 3
    resources:
      - { name: coredns, kind: Deployment, namespace: kube-system, resourceType: compute }
```

Consumers read the composed Objects, not the claim status: each carries labels
`platform.playground.io/inventory=<claim>` and
`platform.playground.io/resource-type=<type>`, and its
`status.atProvider.manifest` is the live resource.
`domain_cloud_resources`'s `observed` module (feature `k8s`) lists them and
terran serves them at `GET /api/cloud-resources`.

## Design notes

- Composition logic is real code (KCL via `function-kcl`), not patch-and-transform
  YAML. The three components deliberately demonstrate the three integration
  styles: operator CR, helm Release, raw k8s Objects.
- **WASM roadmap:** the goal is composition functions compiled to WASM and run
  in-cluster (SpinKube-style runtimes). Crossplane functions are gRPC servers
  packaged as OCI images today; when a WASM function runtime is production-ready
  the KCL step is the seam to swap — the XRD/status contract and SDKs don't change.
- `providers.yaml` grants dev-only cluster-admin to provider service accounts —
  fine for kind, NOT for a shared cluster.
- `platform/dev-env/providerconfigs.yaml` is applied separately from
  `providers.yaml` because ProviderConfig CRDs only exist after the provider
  packages are healthy.
