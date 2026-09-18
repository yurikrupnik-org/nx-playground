# Environment Configuration Guide

This guide explains how to manage environment-specific configurations for the zerg API across different deployment environments.

All of it is declared in `apps/zerg/api/butler.toml` — `[config]` and `[env.<env>.config]` for values, `[env.prod.externalSecret]` for secret references — and rendered by `just k8s-gen` into `k8s/values*.yaml` and `manifests/k8s/apps/zerg-api.yaml`.

## Environment Variables

### Core Configuration
- `PORT`: Server port (default: 8080)
- `RUST_LOG`: Logging level configuration
- `DATABASE_URL`: PostgreSQL connection string
- `REDIS_HOST`: Redis connection URL

### CORS & Auth Redirects
- `CORS_ALLOWED_ORIGIN`: Frontend origin for CORS (e.g., http://localhost:3000)
- `REDIRECT_BASE_URL`: OAuth callback base URL (e.g., http://localhost:8080); the WorkOS redirect URI is `{REDIRECT_BASE_URL}/api/auth/callback`
- `FRONTEND_URL`: Frontend application URL for post-auth redirects (login landing, logout `return_to`)

### Authentication (WorkOS AuthKit via the oidc-auth BFF)
- `WORKOS_CLIENT_ID`: WorkOS environment client id (`client_...`) — required
- `WORKOS_API_KEY`: WorkOS API key (`sk_...`) — required
- `WORKOS_ISSUER`: Token issuer override; defaults to `https://api.workos.com/user_management/{WORKOS_CLIENT_ID}` (set only for a custom auth domain)
- `ZERG_SESSION_COOKIE_NAME` / `ZERG_SESSION_COOKIE_SECURE` / `ZERG_SESSION_TTL_SECS`: Browser session cookie settings (defaults: `zerg_session` / `false` / `28800`)

Google/GitHub social login is brokered by WorkOS — no `GOOGLE_*`/`GITHUB_*` variables are read anymore.

### Feature Flags (Flagsmith)
- `FLAGSMITH_API_URL`: Flagsmith API endpoint
- `FLAGSMITH_ENVIRONMENT_KEY`: Flagsmith environment key

## Understanding Environment Variables in Kubernetes

**Important:** These URLs are used by **your browser**, not by pods talking to each other!

See [DEPLOYMENT_SCENARIOS.md](./DEPLOYMENT_SCENARIOS.md) for detailed explanation of how these work in different setups (port-forward vs ingress vs production).

## Environment-Specific Configuration

### Local Development (Tilt)

For local Kubernetes development with Tilt:

**Ports:**
- Web: http://localhost:5206 (port-forward 5206:8080) — the browser origin; nginx in the
  pod proxies `/api` to `zerg-api:8080`
- API: http://localhost:5221 (port-forward 5221:8080) — direct, bypassing that proxy

**Configuration (`[env.dev.config]`):**
```toml
CORS_ALLOWED_ORIGIN = "http://localhost:5206,https://127.0.0.1.nip.io:8443"
REDIRECT_BASE_URL = "http://localhost:5206"
FRONTEND_URL = "http://localhost:5206"
```

**Usage:**
```bash
tilt up
```

Tilt renders the same `[workload]` the committed manifests come from
(`k8s_yaml(local('kcl run ... -D env=dev'))`), so `[env.dev.config]` is what a local pod
gets — no separate dev overlay to keep in sync.

### Production (GKE)

For production deployments on Google Kubernetes Engine:

**Configuration (`[env.prod.config]`):**
```toml
# ${GATEWAY_SUFFIX} is substituted by Flux (postBuild), not by butler or KCL
CORS_ALLOWED_ORIGIN = "https://zerg.${GATEWAY_SUFFIX}"
REDIRECT_BASE_URL = "https://zerg.${GATEWAY_SUFFIX}"
FRONTEND_URL = "https://zerg.${GATEWAY_SUFFIX}"
```

## Security Best Practices

### Where secrets come from

No secret value is ever written in `butler.toml`. Two homes, one per environment:

1. **Local dev** — placeholder literals in `manifests/k8s/dev/app-secrets.yaml`
   (Secret `zerg-api-secrets`), pulled into the generated aggregate through the root
   `butler.toml` `[k8s] extraResources` list. Put your own WorkOS values there.
2. **Prod** — an `ExternalSecret` of the same name, rendered from
   `[env.prod.externalSecret]` (below). Nothing plaintext is committed.

Either way the pod reads them identically, because the app states the mount once and the
name is all it depends on:

```toml
# apps/zerg/api/butler.toml — later sources win, so app secrets override zerg-wide ones
[[workload.envFrom]]
kind = "secret"
name = "zerg-shared-secrets"

[[workload.envFrom]]
kind = "secret"
name = "zerg-api-secrets"
```

### External Secrets Operator (prod)

`[env.prod.externalSecret]` names the remote keys; the store (`gcp-secret-manager`) and
the 1h refresh are package defaults, so they are not restated:

```toml
# apps/zerg/api/butler.toml
[env.prod.externalSecret]
items = [
    { secretKey = "WORKOS_CLIENT_ID", key = "app-secrets", property = "zerg.workos_client_id" },
    { secretKey = "WORKOS_API_KEY", key = "app-secrets", property = "zerg.workos_api_key" },
]
```

`butler k8s gen` + the `app` package turn that into:

```yaml
apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata:
  name: zerg-api-secrets
  namespace: zerg
spec:
  refreshInterval: 1h
  secretStoreRef:
    name: gcp-secret-manager
    kind: ClusterSecretStore
  target:
    name: zerg-api-secrets
    creationPolicy: Owner
  data:
    - secretKey: WORKOS_CLIENT_ID
      remoteRef:
        key: app-secrets
        property: zerg.workos_client_id
    - secretKey: WORKOS_API_KEY
      remoteRef:
        key: app-secrets
        property: zerg.workos_api_key
```

## WorkOS Setup

1. Go to the [WorkOS Dashboard](https://dashboard.workos.com/) → Applications
2. Copy the client id (`client_...`) and API key (`sk_...`) into the secrets above
3. In the application's **Redirects** tab, register:
   - Sign-in redirect URI: `{REDIRECT_BASE_URL}/api/auth/callback`
     (local: `http://localhost:5206/api/auth/callback`)
   - Sign-out redirect: `{FRONTEND_URL}/login`
   - App homepage URL: `{FRONTEND_URL}`
4. Enable the authentication methods you need (Password, Google OAuth, GitHub OAuth) —
   social providers are brokered by WorkOS, no Google/GitHub console setup in this repo

## Deploying Configuration Changes

Every environment starts the same way: edit `apps/zerg/api/butler.toml`, then regenerate.
`just k8s-check` (part of `just verify`) fails if you commit one without the other.

```bash
just k8s-gen-app zerg_api   # this app only (nx project name, not the image name)
just k8s-gen                # every app + the aggregate kustomization
```

### Dev (Tilt)
```bash
# Tilt re-renders k8s from butler.toml's output on save
tilt up
```

### Production (kubectl)
```bash
# Apply this app alone…
kubectl apply -f manifests/k8s/apps/zerg-api.yaml

# …or every app plus the dev-only Secret fixtures, as `just todo-apply` does
kubectl apply -k manifests/k8s/apps

# Verify deployment
kubectl get pods -n zerg
kubectl logs -n zerg deployment/zerg-api
```

### Production (ArgoCD)
Configuration changes are automatically synchronized when committed to the main branch.
What is synchronized is the **generated** output under `manifests/k8s/apps`, so a
`butler.toml` edit committed without `just k8s-gen` deploys nothing.

## Troubleshooting

### Check environment variables in running pod:
```bash
kubectl exec -n zerg deployment/zerg-api -- env | grep -E "REDIRECT|FRONTEND|CORS|WORKOS"
```

### View logs:
```bash
kubectl logs -n zerg deployment/zerg-api -f
```

### Test the auth flow:
```bash
# Local — expect a 303 redirect to api.workos.com/user_management/authorize
curl -sI http://localhost:5221/api/auth/login | grep -i location

# Production
curl -sI https://api.your-domain.com/api/auth/login | grep -i location
```
