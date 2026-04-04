"""
Preset: NX Monorepo with Rust + Bun
Default Tilt config for NX-based monorepos.
"""

import ..schema as s

# =============================================================================
# Shared defaults for NX Rust monorepos
# =============================================================================

_RUST_DOCKERFILE = "manifests/dockers/rust.Dockerfile"
_NGINX_DOCKERFILE = "manifests/dockers/Dockerfile"

_rust_only = [
    "Cargo.toml"
    "Cargo.lock"
    "apps/"
    "libs/"
]

_rust_ignore = [
    "target/"
    "**/*.md"
    "**/.git/"
    "**/k8s/"
    "**/Tiltfile"
    "apps/**/node_modules/"
]

# =============================================================================
# Helper: quick Rust service
# =============================================================================

rust_service = lambda name: str, pkg: str, app_path: str, port_forwards: [s.PortForwardSpec] = [], deps: [str] = [], labels: [s.Label] = ["backend"], registry: str = "" -> s.RustService {
    _img_name = "{}/{}".format(registry, name) if registry else name
    s.RustService {
        name = name
        package_name = pkg
        app_path = app_path
        image = s.Image {
            name = _img_name
            dockerfile = _RUST_DOCKERFILE
            target = "rust"
            build_args = {APP_NAME = pkg}
            only = _rust_only
            ignore = _rust_ignore
        }
        port_forwards = port_forwards
        resource_deps = deps
        labels = labels
    }
}

# =============================================================================
# Helper: quick web app
# =============================================================================

web_app = lambda name: str, app_path: str, pm: str = "bun", port_forwards: [s.PortForwardSpec] = [], watch_paths: [str] = [], registry: str = "" -> s.WebApp {
    _img_name = "{}/{}".format(registry, name) if registry else name
    _dist = "{}/dist".format(app_path)
    s.WebApp {
        name = name
        build_cmd = "{} nx run {}:build".format(pm, name)
        dist_path = _dist
        app_path = app_path
        image = s.Image {
            name = _img_name
            dockerfile = _NGINX_DOCKERFILE
            target = "nginx"
            build_args = {DIST_PATH = _dist}
        }
        port_forwards = port_forwards
        watch_paths = watch_paths or ["{}/src".format(app_path), "{}/project.json".format(app_path)]
    }
}

# =============================================================================
# Helper: quick port-forward
# =============================================================================

port_forward = lambda name: str, cmd: str, check_port: int -> s.PortForwardResource {
    s.PortForwardResource {
        name = name
        serve_cmd = cmd
        readiness = s.ReadinessProbe {exec_cmd = "nc -z localhost {}".format(check_port)}
    }
}

# =============================================================================
# Helper: quick shared config
# =============================================================================

shared_config = lambda name: str, path: str, objects: [str] -> s.SharedConfig {
    s.SharedConfig {
        name = name
        kustomize_path = path
        objects = objects
    }
}
