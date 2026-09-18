# Service mesh — verdict: not adopted (2026-09-19)

Question: does this repo need a service mesh (Istio) for the traffic it
actually runs, and did it ever have one?

This is a **drift-audit verdict, not a scored evaluation**. Istio had been
listed in `README.md` as a prerequisite ("Service mesh (for gateway /
observability)") with an `Istio Gateway` and a `Kiali` port-forward row, and
none of it existed. Nothing was ever installed, so there is nothing to score
against — the registry row is `rejected` and this doc records why, plus the
conditions that would force a real evaluation.

## Evidence (four classes, per `skill://cncf-manager`)

| class | finding |
|---|---|
| install | none. No `istioctl`, no helm release, no `IstioOperator`, no `istio-system` namespace. `scripts/just/k8s.just` installs CNPG + the Atlas operator; `scripts/just/platform.just` installs Crossplane providers/functions/XRDs; `devkit.toml` adds the Gateway API CRDs. That is every installer in the repo. |
| API objects | none live. `manifests/k8s/base/namespace.yaml` carries `istio-injection: enabled` on the `gateway` and `zerg` namespaces and a commented-out `AuthorizationPolicy` + `DestinationRule`. The labels are inert without a control plane, and that file exists to stay byte-identical to gitops-v1's `infrastructure/platform/networking-config/base/namespace.yaml` so a later Flux apply is a no-op — **do not delete them**. |
| runtime | unverifiable: no cluster was reachable during the audit. |
| gate | none. Nothing would have gone red if a mesh had silently disappeared — which is exactly how the claim survived. |

Progressive delivery: also absent. No Flagger, no Argo Rollouts, no
`VirtualService`, and no `weight:` on any `backendRef` — there is no canary,
blue/green or traffic-splitting mechanism anywhere in the repo.

## What is actually in place

- **North–south**: Gateway API `HTTPRoute`s — `manifests/k8s/base/gateway/`
  (dev, `127.0.0.1.nip.io`) and per-app
  `[[env.prod.workload.extraManifests]]` in `apps/zerg/web/butler.toml` /
  `apps/terran/web/butler.toml`. All parent to `main-gateway` in the `gateway`
  namespace — a Gateway this repo never defines. The CRDs come from
  `devkit.toml`, pinned to v1.2.1 to match what Flux ships; the Gateway itself
  and whatever implements it belong to gitops-v1 (registry status `external`).
- **East–west**: plain ClusterIP Services, axum/tonic clients, tower
  middleware. No sidecars, no mTLS between pods, no L7 policy.
- **Observability**: in-process — `metrics-exporter-prometheus` on three apps
  and the OpenTelemetry SDK exporting OTLP to `otel-collector.monitoring.svc`
  (a collector this repo also does not install; see the registry).

## Why no mesh, for now

The mesh feature set maps onto problems this repo does not have yet: there are
no cross-tenant namespaces needing mTLS, no multi-cluster traffic, and the
per-pod sidecar tax plus a control plane is real operational surface against a
stack whose entire cluster story is one kind cluster and a gitops repo. Tracing
context already exists in-process via OTLP, which is the observability half of
the original claim.

**Canary does not require a mesh.** Gateway API supports weighted
`backendRefs` in the already-installed `standard` channel; automation on top
(Flagger, Argo Rollouts) is a separate decision from a data plane.

## What would reverse this

Any one of these turns the mesh into a scored evaluation under
`skill://cncf-manager` (candidates: Istio ambient, Linkerd, Cilium mesh, *and*
`do nothing`):

1. mTLS between namespaces becomes a compliance requirement rather than a nice
   to have;
2. authorization policy has to move out of the apps (today: `oidc-auth` +
   tower layers per service);
3. traffic needs to span clusters, or per-request routing that Gateway API's
   `HTTPRoute` cannot express;
4. progressive delivery needs automated analysis/rollback — and even then,
   score Argo Rollouts (no data plane) against a mesh, not instead of it.

Prerequisite for any of the above: a cluster gate. Today nothing in `just
verify` or CI starts a cluster (registry gap `no-cluster-in-ci`), so a mesh
would be adopted blind — the same condition that let the Istio claim stand.
