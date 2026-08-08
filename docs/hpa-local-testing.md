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

### Example HPA Manifest

Located at `apps/zerg/api/k8s/kustomize/base/hpa.yaml`:

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: zerg-api
  labels:
    app: zerg-api
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: zerg-api
  minReplicas: 2
  maxReplicas: 10
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70
    - type: Resource
      resource:
        name: memory
        target:
          type: Utilization
          averageUtilization: 80
  behavior:
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
        - type: Percent
          value: 10
          periodSeconds: 60
    scaleUp:
      stabilizationWindowSeconds: 0
      policies:
        - type: Percent
          value: 100
          periodSeconds: 15
        - type: Pods
          value: 4
          periodSeconds: 15
      selectPolicy: Max
```

### Key Configuration Explained

| Setting | Value | Purpose |
|---------|-------|---------|
| `minReplicas` | 2 | Minimum pods for high availability |
| `maxReplicas` | 10 | Maximum pods to prevent resource exhaustion |
| `cpu.averageUtilization` | 70% | Scale up when average CPU exceeds 70% |
| `memory.averageUtilization` | 80% | Scale up when average memory exceeds 80% |
| `scaleDown.stabilizationWindowSeconds` | 300 | Wait 5 minutes before scaling down (prevents flapping) |
| `scaleUp.stabilizationWindowSeconds` | 0 | Scale up immediately when needed |

## Monitoring HPA

### Check HPA Status

```bash
kubectl get hpa -n zerg
```

Example output:
```
NAME         REFERENCE               TARGETS                       MINPODS   MAXPODS   REPLICAS   AGE
zerg-api     Deployment/zerg-api     cpu: 0%/70%, memory: 7%/80%   2         10        2          25m
zerg-tasks   Deployment/zerg-tasks   cpu: 0%/70%, memory: 7%/80%   2         10        2          25m
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

2. Verify resource requests are set in deployment:
   ```yaml
   resources:
     requests:
       memory: "128Mi"
       cpu: "250m"
   ```

   HPA cannot calculate utilization percentage without resource requests.

### Pods Scaling Too Aggressively

Increase stabilization window or adjust thresholds:
```yaml
behavior:
  scaleDown:
    stabilizationWindowSeconds: 600  # 10 minutes
```

## Database Connection Considerations

When using HPA with database-connected services, be aware of connection pool limits.

### Problem

With default PostgreSQL `max_connections=100`:
- 10 pods × 20 connections/pod = 200 connections (exceeds limit!)

### Solution

Reduce connection pool size per pod:
```yaml
env:
  - name: DB_MAX_CONNECTIONS
    value: "10"
  - name: DB_MIN_CONNECTIONS
    value: "2"
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

## Files Modified/Created for HPA

- `apps/zerg/api/k8s/kustomize/base/hpa.yaml` - HPA for zerg-api
- `apps/zerg/api/k8s/kustomize/base/kustomization.yaml` - Added hpa.yaml to resources
- `apps/zerg/tasks/k8s/kustomize/base/hpa.yaml` - HPA for zerg-tasks
- `apps/zerg/tasks/k8s/kustomize/base/kustomization.yaml` - Added hpa.yaml to resources
- `apps/zerg/*/k8s/kustomize/overlays/dev/kustomization.yaml` - Reduced DB pool sizes
