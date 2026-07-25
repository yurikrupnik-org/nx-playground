# Environment Configuration Guide

This guide explains how to manage environment-specific configurations for the Terran API across different deployment environments.

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
- API: http://localhost:5201 (port-forward 5201:8080)
- Web: http://localhost:5206 (port-forward 5206:80)

**Configuration (dev overlay):**
```yaml
CORS_ALLOWED_ORIGIN: "http://localhost:5206"
REDIRECT_BASE_URL: "http://localhost:5201"
FRONTEND_URL: "http://localhost:5206"
```

**Usage:**
```bash
tilt up
```

The dev overlay automatically configures all environment variables for local development.

### Production (GKE)

For production deployments on Google Kubernetes Engine:

**Configuration (prod overlay):**
```yaml
CORS_ALLOWED_ORIGIN: "https://your-production-domain.com"
REDIRECT_BASE_URL: "https://api.your-production-domain.com"
FRONTEND_URL: "https://your-production-domain.com"
```

## Security Best Practices

### Using Kubernetes Secrets

For production, sensitive values should be stored in Kubernetes Secrets:

1. **Create a secret from the template:**
   ```bash
   cd k8s/kustomize/overlays/prod
   cp secrets.example.yaml secrets.yaml
   # Edit secrets.yaml with your actual values
   ```

2. **Apply the secret:**
   ```bash
   kubectl apply -f secrets.yaml
   ```

3. **Update kustomization.yaml to use secrets:**
   ```yaml
   # Replace direct value:
   - name: WORKOS_API_KEY
     value: "CHANGE-ME-use-k8s-secret"

   # With secret reference:
   - name: WORKOS_API_KEY
     valueFrom:
       secretKeyRef:
         name: zerg-api-secrets
         key: WORKOS_API_KEY
   ```

### External Secrets Operator (Recommended)

For better secret management, use External Secrets Operator with cloud secret managers:

**GCP Secret Manager** (see `kustomize/overlays/{dev,prod}/external-secret.yaml`):
```yaml
apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata:
  name: zerg-api-secrets
spec:
  refreshInterval: 1h
  secretStoreRef:
    name: gcp-secret-manager
    kind: ClusterSecretStore
  target:
    name: zerg-api-secrets
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
     (local: `http://localhost:5201/api/auth/callback`)
   - Sign-out redirect: `{FRONTEND_URL}/login`
   - App homepage URL: `{FRONTEND_URL}`
4. Enable the authentication methods you need (Password, Google OAuth, GitHub OAuth) —
   social providers are brokered by WorkOS, no Google/GitHub console setup in this repo

## Deploying Configuration Changes

### Dev (Tilt)
```bash
# Configuration updates are applied automatically with Tilt
tilt up
```

### Production (kubectl)
```bash
# Apply the production configuration
kubectl apply -k k8s/kustomize/overlays/prod

# Verify deployment
kubectl get pods -n zerg
kubectl logs -n zerg deployment/zerg-api
```

### Production (ArgoCD)
Configuration changes are automatically synchronized when committed to the main branch.

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
curl -sI http://localhost:5201/api/auth/login | grep -i location

# Production
curl -sI https://api.your-domain.com/api/auth/login | grep -i location
```
