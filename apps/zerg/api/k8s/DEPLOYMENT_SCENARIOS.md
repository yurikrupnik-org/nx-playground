# Deployment Scenarios and Environment Configuration

This document explains how environment variables like `FRONTEND_URL`, `REDIRECT_BASE_URL`, and `CORS_ALLOWED_ORIGIN` work in different deployment scenarios.

**Where they are set.** An app's own `butler.toml`: `[config]` for the keys that hold in every environment, `[env.<env>.config]` for the ones that do not. `just k8s-gen` merges them into `k8s/values.yaml` + `k8s/values.<env>.yaml` and renders the ConfigMap the `app` KCL package names `<app>-config`, which the Deployment mounts through `envFrom`. There are no per-app kustomize overlays any more, so every snippet below is TOML in `apps/zerg/api/butler.toml`. Variable-by-variable reference: [ENV_CONFIG.md](./ENV_CONFIG.md).

## The Challenge

These environment variables contain URLs that the **user's browser** needs to access, but they're configured **inside a Kubernetes pod**. This creates different requirements depending on how you access the services.

## Scenario 1: Tilt with Port-Forwards (Current Dev Setup)

### How It Works

```
┌─────────────────────┐
│  Developer Machine  │
│                     │
│  Browser            │
│  ↓                  │
│  localhost:5206 ────┼────► port-forward ────► zerg-web pod:8080
│  localhost:5221 ────┼────► port-forward ────► zerg-api pod:8080
│                     │
└─────────────────────┘
```

### Configuration

```toml
# apps/zerg/api/butler.toml (current)
[env.dev.config]
CORS_ALLOWED_ORIGIN = "http://localhost:5206,https://127.0.0.1.nip.io:8443"
REDIRECT_BASE_URL = "http://localhost:5206"
FRONTEND_URL = "http://localhost:5206"
```

### Why This Works

1. Port-forwards tunnel cluster services to your localhost
2. Browser loads web app from `http://localhost:5206`
3. The SPA calls `/api/...` on its own origin; nginx inside the web pod proxies that to `zerg-api:8080`
4. OAuth redirects go to `http://localhost:5206/api/auth/callback` — same origin, proxied to the API pod
5. After the callback establishes the session cookie, it redirects to `http://localhost:5206/tasks`

### Pros
- ✅ Simple - just `tilt up`
- ✅ No DNS configuration needed
- ✅ Works on any machine

### Cons
- ❌ Only one developer can use these ports at a time
- ❌ Port conflicts if running multiple projects
- ❌ Doesn't work for pod-to-pod communication
- ❌ Confusing (localhost inside K8s?)

### Usage

```bash
tilt up
# Access at http://localhost:5206
```

---

## Scenario 2: Ingress with Local DNS (Better Dev Setup)

### How It Works

```
┌─────────────────────┐
│  Developer Machine  │
│                     │
│  Browser            │
│  ↓                  │
│  zerg.local ────────┼────► Ingress ────► zerg-web service:8080 ──► pod:8080
│  api.zerg.local ────┼────► Ingress ────► zerg-api service:8080 ──► pod:8080
│                     │
└─────────────────────┘
```

### Configuration

```toml
# apps/zerg/api/butler.toml
[env.dev.config]
CORS_ALLOWED_ORIGIN = "http://zerg.local"
REDIRECT_BASE_URL = "http://api.zerg.local"
FRONTEND_URL = "http://zerg.local"
```

### Setup Required

This repo already ships the Gateway API version of this scenario —
`manifests/k8s/base/gateway/{zerg-api,zerg-web}.yaml` route `127.0.0.1.nip.io` at
`main-gateway`, which needs no `/etc/hosts` entry (that is why the dev
`CORS_ALLOWED_ORIGIN` above also lists `https://127.0.0.1.nip.io:8443`). The
ingress-nginx path below is the alternative if you want your own hostnames.

**1. Install Ingress Controller:**
```bash
kubectl apply -f https://raw.githubusercontent.com/kubernetes/ingress-nginx/controller-v1.8.1/deploy/static/provider/cloud/deploy.yaml
```

**2. Add to `/etc/hosts`:**
```bash
echo "127.0.0.1 zerg.local api.zerg.local" | sudo tee -a /etc/hosts
```

**3. Create Ingress:**

An object like this is not workload shape, so it goes in the app's
`[[env.<env>.workload.extraManifests]]` list — verbatim pass-through, which is why it
carries its own name and namespace (`apps/zerg/web/butler.toml` does exactly this for
its prod HTTPRoute):

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: zerg
  namespace: zerg
spec:
  ingressClassName: nginx
  rules:
  - host: zerg.local
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: zerg-web
            port:
              number: 8080
  - host: api.zerg.local
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: zerg-api
            port:
              number: 8080
```

**4. Port-forward Ingress Controller:**
```bash
kubectl port-forward -n ingress-nginx service/ingress-nginx-controller 80:80 443:443
```

### Pros
- ✅ Real domain names (easier to understand)
- ✅ Multiple developers can use different domains
- ✅ Closer to production setup
- ✅ Can add TLS easily

### Cons
- ❌ Requires ingress controller
- ❌ Need to edit /etc/hosts
- ❌ Still need port-forward for ingress controller

### Usage

```bash
# Regenerate the manifests after adding the extraManifests entry, then apply
just k8s-gen
kubectl apply -k manifests/k8s/apps

# Port-forward ingress controller
kubectl port-forward -n ingress-nginx service/ingress-nginx-controller 80:80

# Access at http://zerg.local
```

---

## Scenario 3: Production GKE with Load Balancer

### How It Works

```
┌──────────────┐
│   Internet   │
│      ↓       │
│  Public IP   │
│      ↓       │
│ Load Balancer│ ─────► Ingress ─────► Services ─────► Pods
│  (GCP LB)    │
└──────────────┘
```

### Configuration

```toml
# apps/zerg/api/butler.toml — ${GATEWAY_SUFFIX} is substituted by Flux (postBuild),
# not by butler or KCL. One hostname: the SPA owns it and /api is proxied inside the
# web pod, so the two-host Ingress illustration below is the alternative shape.
[env.prod.config]
CORS_ALLOWED_ORIGIN = "https://zerg.${GATEWAY_SUFFIX}"
REDIRECT_BASE_URL = "https://zerg.${GATEWAY_SUFFIX}"
FRONTEND_URL = "https://zerg.${GATEWAY_SUFFIX}"
```

### Setup Required

**1. Configure DNS:**
```
zerg.yourdomain.com     → Load Balancer IP
api.zerg.yourdomain.com → Load Balancer IP
```

**2. Create Ingress with TLS:**

What this repo actually deploys is a Gateway API `HTTPRoute` on the shared
`main-gateway` (TLS terminates there), declared in
`apps/zerg/web/butler.toml` under `[[env.prod.workload.extraManifests]]`. An
equivalent nginx Ingress, in the same `extraManifests` list, would read:

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: zerg
  namespace: zerg
  annotations:
    cert-manager.io/cluster-issuer: "letsencrypt-prod"
spec:
  ingressClassName: nginx
  tls:
  - hosts:
    - zerg.yourdomain.com
    - api.zerg.yourdomain.com
    secretName: zerg-tls
  rules:
  - host: zerg.yourdomain.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: zerg-web
            port:
              number: 8080
  - host: api.zerg.yourdomain.com
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: zerg-api
            port:
              number: 8080
```

**3. Configure the IdP:**

Social login is brokered by WorkOS, so there is no Google/GitHub console step. In the
WorkOS dashboard's **Redirects** tab register
`https://zerg.yourdomain.com/api/auth/callback` — `{REDIRECT_BASE_URL}/api/auth/callback`,
the one path the API builds (`apps/zerg/api/src/config.rs::callback_url`).

### Pros
- ✅ Production-ready
- ✅ TLS/HTTPS
- ✅ Real domain names
- ✅ Auto-scaling with GKE

### Cons
- ❌ Costs money (load balancer, etc.)
- ❌ Need real domain
- ❌ DNS propagation time

---

## Scenario 4: In-Cluster Communication

### Use Case
If the API needs to make HTTP calls to the frontend (unlikely but possible).

### Configuration

Use Kubernetes service DNS:

```yaml
FRONTEND_URL: "http://zerg-web.zerg.svc.cluster.local"
```

**Problem:** This won't work for OAuth redirects because the browser can't resolve cluster DNS!

**Solution:** Use different variables for different purposes:

```rust
// In your Rust code
pub fn get_frontend_url_external() -> String {
    env::var("FRONTEND_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

pub fn get_frontend_url_internal() -> String {
    env::var("FRONTEND_URL_INTERNAL")
        .unwrap_or_else(|_| "http://zerg-web.zerg.svc.cluster.local".to_string())
}
```

```toml
# In butler.toml [env.dev.config]
FRONTEND_URL = "http://localhost:5206"  # For browser redirects
FRONTEND_URL_INTERNAL = "http://zerg-web.zerg.svc.cluster.local"  # For pod-to-pod
```

---

## Comparison Table

| Scenario | Access From | URLs Used | Setup Complexity | Best For |
|----------|-------------|-----------|------------------|----------|
| **Port-Forward** | localhost:5221/5206 | localhost | Low | Quick dev, single developer |
| **Local Ingress** | zerg.local | Custom domains | Medium | Team dev, realistic setup |
| **GKE Production** | yourdomain.com | Public domains | High | Production |
| **Pod-to-Pod** | N/A | cluster.local | Low | Internal services |

---

## Recommendations

### For Development (Tilt)

**Option A: Keep it simple (current)**
```toml
# Use port-forwards and localhost
[env.dev.config]
CORS_ALLOWED_ORIGIN = "http://localhost:5206,https://127.0.0.1.nip.io:8443"
REDIRECT_BASE_URL = "http://localhost:5206"
FRONTEND_URL = "http://localhost:5206"
```

**Option B: Use local ingress**
```toml
# Set up ingress with local DNS
[env.dev.config]
CORS_ALLOWED_ORIGIN = "http://zerg.local"
REDIRECT_BASE_URL = "http://api.zerg.local"
FRONTEND_URL = "http://zerg.local"
```

### For Production

```toml
[env.prod.config]
# Single hostname, matching Scenario 3
CORS_ALLOWED_ORIGIN = "https://zerg.yourdomain.com"
REDIRECT_BASE_URL = "https://zerg.yourdomain.com"
FRONTEND_URL = "https://zerg.yourdomain.com"
```

### Environment Variable Naming

Consider using clearer names:

```toml
# External URLs (for browser)
BROWSER_FRONTEND_URL = "http://localhost:5206"
BROWSER_API_URL = "http://localhost:5221"

# Internal URLs (for pod-to-pod, if needed)
INTERNAL_FRONTEND_URL = "http://zerg-web.zerg.svc.cluster.local"
INTERNAL_API_URL = "http://zerg-api.zerg.svc.cluster.local:8080"
```

---

## Common Pitfalls

### 1. Using cluster DNS for browser URLs
```toml
❌ FRONTEND_URL = "http://zerg-web.zerg.svc.cluster.local"
```
Browser can't resolve this!

### 2. Using localhost in production
```toml
❌ REDIRECT_BASE_URL = "http://localhost:8080"  # in prod
```
Users' browsers aren't on the same machine as your cluster!

### 3. Wrong redirect URI registered with WorkOS
The sign-in redirect URI is always `{REDIRECT_BASE_URL}/api/auth/callback` — there is no
`/oauth/<provider>/callback` route. Register both:
- Dev: `http://localhost:5206/api/auth/callback`
- Prod: `https://zerg.yourdomain.com/api/auth/callback`

---

## Testing Your Configuration

```bash
# Check environment variables in pod
kubectl exec -n zerg deployment/zerg-api -- env | grep -E "FRONTEND|REDIRECT|CORS"

# Test the auth flow — expect a 303 to api.workos.com/user_management/authorize
# Dev (through the web pod's /api proxy):
curl -sI http://localhost:5206/api/auth/login | grep -i location

# Dev (API port-forward directly):
curl -sI http://localhost:5221/api/auth/login | grep -i location

# Prod:
curl -sI https://zerg.yourdomain.com/api/auth/login | grep -i location
```

---

## Switching Between Scenarios

To switch from port-forward to ingress:

1. Change the three URLs in `apps/zerg/api/butler.toml` `[env.dev.config]` (Option B above).
2. Add the Ingress (or HTTPRoute) object to `[[env.dev.workload.extraManifests]]` in the
   app whose hostname it is — `apps/zerg/web/butler.toml` for the SPA.
3. Run `just k8s-gen`, then `kubectl apply -k manifests/k8s/apps`.
4. Drop `[tilt] hostPort` from that app's `butler.toml` if you no longer want the port
   forward, and run `just tilt-gen`. Do not edit the Tiltfile: it is generated output
   (`just tilt-check` fails on a hand edit), and the `k8s_yaml(local('kcl run ...'))`
   line in it renders the same `[workload]` Tilt-side.
