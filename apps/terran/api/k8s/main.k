"""
terran-api Kubernetes manifests (KCL) — single source of truth.

Render:  kcl run apps/terran/api/k8s -D env=dev   [-D image=<ref>]
"""
import manifests
import app

_env = option("env") or "dev"
_dev = _env == "dev"
_image = option("image") or ("yurikrupnik/terran-api" if _dev else "europe-west1-docker.pkg.dev/yk-artifact-registry/docker/terran-api:main")
_ns = option("namespace") or "terran"

_config = {
    PORT = "8080"
    RUST_LOG = "info"
}

_w = app.Workload {
    name = "terran-api"
    namespace = _ns
    partOf = "terran"
    component = "api-server"
    image = _image
    port = 8080
    prometheus = False
    startupProbe = {$type = "http", path = "/health", port = 8080, initialDelaySeconds = 2, periodSeconds = 5, failureThreshold = 6, timeoutSeconds = 3}
    livenessProbe = {$type = "http", path = "/health", port = 8080, periodSeconds = 10, failureThreshold = 3, timeoutSeconds = 3}
    readinessProbe = {$type = "http", path = "/ready", port = 8080, periodSeconds = 5, failureThreshold = 3, timeoutSeconds = 2}
    resources = {requests = {memory = "128Mi", cpu = "250m"}, limits = {memory = "512Mi"}}
    envFrom = [
        {kind = "configMap", name = "terran-api-config"}
        {kind = "secret", name = "terran-api-secrets"}
    ]
    hpa = {minReplicas = 1, maxReplicas = 10, cpu = 70, memory = 80}
}

_manifests = app.render(_w) \
    + [app.configMap("terran-api-config", _config, _ns)]

manifests.yaml_stream(_manifests)
