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

Same surface everywhere: `create / get / connection / delete`.

| Language | Location | Run the example |
|---|---|---|
| Rust | `libs/platform/devenv-sdk` (workspace member) | `cargo run -p devenv-sdk --example status -- demo` |
| Python | `platform/sdk/python` | `uv run platform/sdk/python/example.py demo` |
| Node | `platform/sdk/node` | `node example.mjs demo` (after `bun install`) |

**Node caveat:** run examples with `node`, not `bun` — `@kubernetes/client-node`
supplies the cluster CA and client certs through a custom https agent, which
bun's fetch ignores (TLS verification + client auth both fail).

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
