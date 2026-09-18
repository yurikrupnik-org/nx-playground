# Testing HPA (Horizontal Pod Autoscaler) Locally with Kind

This guide documents how to set up and test HPA in a local Kind cluster.

## Prerequisites

- Kind cluster running
- Tilt for local development
- `kubectl` configured
- `wrk` for benchmarking

## Required Cluster Components

### Metrics Server

HPA requires the Kubernetes Metrics Server to collect CPU/memory metrics from pods. This is **not installed by default** in Kind.

#### Install Metrics Server

```bash
kubectl apply -f https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml
```

#### Patch for Kind (required)

Kind uses self-signed certificates, so metrics-server needs to skip TLS verification:

```bash
kubectl patch deployment metrics-server -n kube-system \
  --type='json' \
  -p='[{"op": "add", "path": "/spec/template/spec/containers/0/args/-", "value": "--kubelet-insecure-tls"}]'
```

#### Verify Installation

```bash
# Wait for metrics-server to be ready
kubectl rollout status deployment/metrics-server -n kube-system

# Test metrics collection (wait ~30 seconds after install)
kubectl top nodes
kubectl top pods -n zerg
```

## HPA Configuration

### Where it is declared

`apps/zerg/api/butler.toml` and `apps/zerg/tasks/butler.toml`. The whole app-side
surface is the replica floor and ceiling — the utilization targets are the `app` KCL
package's own defaults, so they are not restated:

```toml
[workload.hpa]
minReplicas = 1
maxReplicas = 10
```

There is no `hpa.yaml` to edit: `just k8s-gen` (or `just k8s-gen-app zerg_api`) renders
that table into `manifests/k8s/apps/zerg-api.yaml`, and `just k8s-check` fails if the two
have drifted. The package also omits `spec.replicas` from the Deployment whenever an
autoscaler owns it, so the kind-wide `replicas = 1` default never fights the HPA.

### Rendered Manifest

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  labels:
    app: zerg-api
  name: zerg-api
  namespace: zerg
spec:
  behavior:
    scaleDown:
      policies:
      - periodSeconds: 60
        type: Percent
        value: 10
      stabilizationWindowSeconds: 300
    scaleUp:
      policies:
      - periodSeconds: 15
        type: Percent
        value: 100
      - periodSeconds: 15
        type: Pods
        value: 4
      selectPolicy: Max
      stabilizationWindowSeconds: 0
  maxReplicas: 10
  metrics:
  - resource:
      name: cpu
      target:
        averageUtilization: 70
        type: Utilization
    type: Resource
  - resource:
      name: memory
      target:
        averageUtilization: 80
        type: Utilization
    type: Resource
  minReplicas: 1
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: zerg-api
```

### Key Configuration Explained

| Setting | Value | Purpose | Where it is set |
|---------|-------|---------|-----------------|
| `minReplicas` | 1 | Replica floor; raise it for real high availability | `[workload.hpa]` |
| `maxReplicas` | 10 | Maximum pods to prevent resource exhaustion | `[workload.hpa]` |
| `cpu.averageUtilization` | 70% | Scale up when average CPU exceeds 70% | `[workload.hpa] cpu` — package default |
| `memory.averageUtilization` | 80% | Scale up when average memory exceeds 80% | `[workload.hpa] memory` — package default |
| `scaleDown.stabilizationWindowSeconds` | 300 | Wait 5 minutes before scaling down (prevents flapping) | fixed by the package |
| `scaleUp.stabilizationWindowSeconds` | 0 | Scale up immediately when needed | fixed by the package |

## Monitoring HPA

### Check HPA Status

```bash
kubectl get hpa -n zerg
```

Example output:
```
NAME         REFERENCE               TARGETS                       MINPODS   MAXPODS   REPLICAS   AGE
zerg-api     Deployment/zerg-api     cpu: 0%/70%, memory: 7%/80%   1         10        1          25m
zerg-tasks   Deployment/zerg-tasks   cpu: 0%/70%, memory: 7%/80%   1         10        1          25m
```

If targets show `<unknown>`, metrics-server isn't working properly.

### Watch HPA in Real-Time

```bash
kubectl get hpa -n zerg -w
```

### Check Pod Resource Usage

```bash
kubectl top pods -n zerg
```

## Testing HPA Scaling

### Generate Load with wrk

```bash
# Quick benchmark (10s, light load)
just bench-cluster-quick

# Full benchmark (30s, heavier load)
just bench-cluster-all
```

### Manual Load Test

```bash
# Generate sustained load
wrk -t4 -c100 -d60s http://localhost:5221/api/tasks
```

### Observe Scaling

In a separate terminal:
```bash
watch -n 2 'kubectl get pods -n zerg && echo "---" && kubectl get hpa -n zerg'
```

## Troubleshooting

### HPA Shows `<unknown>` for Metrics

1. Check if metrics-server is running:
   ```bash
   kubectl get deployment metrics-server -n kube-system
   ```

2. Check metrics-server logs:
   ```bash
   kubectl logs -n kube-system deployment/metrics-server
   ```

3. Ensure the Kind patch was applied (see above)

### Pods Not Scaling Up

1. Check HPA events:
   ```bash
   kubectl describe hpa -n zerg
   ```

2. Verify resource requests are set. They come from the kind-wide shape in the root
   `butler.toml`, so every service already has them:

   ```toml
   [workloadDefaults.service.resources]
   requests = { memory = "128Mi", cpu = "250m" }
   limits = { memory = "512Mi" }
   ```

   HPA cannot calculate utilization percentage without resource requests.

### Pods Scaling Too Aggressively

Raise the utilization targets — the only two knobs `[workload.hpa]` exposes beyond the
replica bounds:

```toml
[workload.hpa]
cpu = 85
memory = 90
```

The `behavior` block (stabilization windows and scale policies) is fixed by the package,
not per-app config. Tuning it means switching that app to the package's KEDA
`[workload.scaler]` table, which takes `behavior` as pass-through — and cannot be
combined with `[workload.hpa]`.

## Database Connection Considerations

When using HPA with database-connected services, be aware of connection pool limits.

### Problem

With default PostgreSQL `max_connections=100`:
- 10 pods × 20 connections/pod = 200 connections (exceeds limit!)

### Solution

The pool size is capped in the zerg-wide ConfigMap, not per app —
`apps/zerg/shared/k8s/kustomize/overlays/dev/kustomization.yaml` (that tree survives the
butler migration because it belongs to no single app and is referenced by the root
`butler.toml` `[[tilt.sharedResource]]`):

```yaml
configMapGenerator:
  - name: zerg-shared-config
    literals:
      - DB_MAX_CONNECTIONS=10
      - DB_MIN_CONNECTIONS=2
```

This allows: 10 pods × 10 connections = 100 (within limit)

For production, consider using **PgBouncer** as a connection pooler.

## Performance Notes

### Local Kind Cluster Limitations

| Factor | Impact |
|--------|--------|
| `kubectl port-forward` | ~30-50% slower (single tunnel bottleneck) |
| Kind/Docker networking | ~10-20% slower (extra network hops) |
| Shared node resources | Variable (all pods compete for same CPU) |

### Benchmark Results (Kind Cluster)

| Endpoint | Requests/sec | Avg Latency | P99 Latency |
|----------|-------------|-------------|-------------|
| gRPC GET | ~2,300 req/s | ~20ms | ~25ms |
| Direct DB GET | ~6,900 req/s | ~9ms | ~85ms |

Real cloud Kubernetes clusters (GKE, EKS, AKS) with proper ingress will perform significantly better.

## Where the HPA Lives

- `apps/zerg/api/butler.toml` — `[workload.hpa]` for zerg-api
- `apps/zerg/tasks/butler.toml` — `[workload.hpa]` for zerg-tasks
- `manifests/k8s/apps/{zerg-api,zerg-tasks}.yaml` — generated output, `just k8s-gen`
- `apps/zerg/shared/k8s/kustomize/overlays/dev/kustomization.yaml` — the reduced DB pool
  sizes every zerg pod inherits through `zerg-shared-config`
