"""
terran-web (nginx static) Kubernetes manifests (KCL) — single source of truth.

Render:  kcl run apps/terran/web/k8s -D env=dev   [-D image=<ref>]
prod adds an HTTPRoute on the shared gateway.
"""
import manifests
import app

_env = option("env") or "dev"
_dev = _env == "dev"
_image = option("image") or ("yurikrupnik/terran-web" if _dev else "europe-west1-docker.pkg.dev/yk-artifact-registry/docker/terran-web:main")
_ns = option("namespace") or "terran"
_gw = "$" + "{GATEWAY_SUFFIX}"

_httproute = {
    apiVersion = "gateway.networking.k8s.io/v1"
    kind = "HTTPRoute"
    metadata = {name = "terran-web", namespace = _ns}
    spec = {
        parentRefs = [{name = "main-gateway", namespace = "gateway", sectionName = "https"}]
        hostnames = ["terran.${_gw}"]
        rules = [{
            matches = [{path = {type = "PathPrefix", value = "/"}}]
            backendRefs = [{name = "terran-web", port = 8080}]
        }]
    }
}

_w = app.Workload {
    name = "terran-web"
    namespace = _ns
    partOf = "terran"
    component = "frontend"
    image = _image
    port = 8080
    runAsUser = 101
    runAsGroup = 101
    prometheus = False
    startupProbe = {type = "http", path = "/health", port = "http", initialDelaySeconds = 2, periodSeconds = 5, failureThreshold = 4, timeoutSeconds = 3}
    livenessProbe = {type = "http", path = "/health", port = "http", periodSeconds = 10, failureThreshold = 3, timeoutSeconds = 3}
    readinessProbe = {type = "http", path = "/health", port = "http", periodSeconds = 5, failureThreshold = 3, timeoutSeconds = 2}
    resources = {requests = {memory = "32Mi", cpu = "50m"}, limits = {memory = "64Mi", cpu = "100m"}}
    env = [
        {name = "UPSTREAM_HOST", configMapName = "proxy-envs", configMapKey = "upstream_host"}
        {name = "UPSTREAM_PORT", configMapName = "proxy-envs", configMapKey = "upstream_port"}
        {name = "UPSTREAM_NAME", configMapName = "proxy-envs", configMapKey = "upstream_name"}
    ]
    volumeMounts = [
        {name = "nginx-conf-d", mountPath = "/etc/nginx/conf.d"}
        {name = "nginx-cache", mountPath = "/var/cache/nginx"}
        {name = "nginx-run", mountPath = "/var/run"}
        {name = "tmp", mountPath = "/tmp"}
    ]
    volumes = [
        {name = "nginx-conf-d", emptyDir = {}}
        {name = "nginx-cache", emptyDir = {}}
        {name = "nginx-run", emptyDir = {}}
        {name = "tmp", emptyDir = {}}
    ]
    terminationGracePeriodSeconds = 10
    extraManifests = [_httproute] if not _dev else []
}

_proxyConfig = {
    upstream_host = "terran-api"
    upstream_port = "8080"
    upstream_name = "backend"
}

_manifests = app.render(_w) \
    + [app.configMap("proxy-envs", _proxyConfig, _ns)]

manifests.yaml_stream(_manifests)
