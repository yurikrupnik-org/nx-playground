"""
Tilt configuration schemas.

Defines typed, validated schemas for Tilt resources.
Used by presets and render functions to generate Tiltfile Starlark code.
"""

# =============================================================================
# Enums
# =============================================================================

type Language = "rust" | "go" | "python" | "bun" | "nodejs"
type RepoType = "nx-monorepo" | "polyrepo"
type Label = "backend" | "frontend" | "bun" | "config" | "port-forward" | "migrations" | "infra" | "static" | "worker"

# =============================================================================
# Sub-schemas
# =============================================================================

schema Image:
    """Docker image build configuration."""
    registry: str = ""
    name: str
    dockerfile: str
    target?: str
    build_args: {str: str} = {}
    context: str = "."
    only: [str] = []
    ignore: [str] = []

    check:
        name, "image name must not be empty"
        dockerfile, "dockerfile path must not be empty"

schema PortForwardSpec:
    local: int
    remote: int

    check:
        1 <= local <= 65535, "local port must be 1-65535"
        1 <= remote <= 65535, "remote port must be 1-65535"

schema LiveUpdateRule:
    local_path: str
    container_path: str

schema ReadinessProbe:
    period_secs: int = 5
    exec_cmd: str

    check:
        period_secs > 0
        exec_cmd, "exec_cmd must not be empty"

# =============================================================================
# Resource schemas
# =============================================================================

schema RustService:
    """Cargo binary deployed to K8s via docker_build + kustomize."""
    name: str
    package_name: str
    app_path: str
    image: Image
    kustomize_path: str = "${app_path}/k8s/kustomize/overlays/dev"
    port_forwards: [PortForwardSpec] = []
    labels: [Label] = ["backend"]
    resource_deps: [str] = []
    live_update: [LiveUpdateRule] = [LiveUpdateRule {
        local_path = "${app_path}/src"
        container_path = "/app/${app_path}/src"
    }]

    check:
        name, "name required"
        package_name, "package_name required"

schema GoService:
    """Go binary deployed to K8s via docker_build + kustomize."""
    name: str
    binary_name: str
    app_path: str
    image: Image
    kustomize_path: str = "${app_path}/k8s/kustomize/overlays/dev"
    port_forwards: [PortForwardSpec] = []
    labels: [Label] = ["backend"]
    resource_deps: [str] = []
    live_update: [LiveUpdateRule] = [LiveUpdateRule {
        local_path = "${app_path}"
        container_path = "/app/${app_path}"
    }]

    check:
        name, "name required"
        binary_name, "binary_name required"

schema PythonService:
    """Python service deployed to K8s via docker_build + kustomize."""
    name: str
    app_path: str
    image: Image
    kustomize_path: str = "${app_path}/k8s/kustomize/overlays/dev"
    port_forwards: [PortForwardSpec] = []
    labels: [Label] = ["backend"]
    resource_deps: [str] = []
    live_update: [LiveUpdateRule] = [LiveUpdateRule {
        local_path = "${app_path}"
        container_path = "/app/${app_path}"
    }]

    check:
        name, "name required"

schema WebApp:
    """Frontend app: local build + nginx/static docker image."""
    name: str
    build_cmd: str
    dist_path: str
    app_path: str
    image: Image
    kustomize_path: str = "${app_path}/k8s/kustomize/overlays/dev"
    port_forwards: [PortForwardSpec] = []
    labels: [Label] = ["bun"]
    resource_deps: [str] = []
    watch_paths: [str] = []

    check:
        name, "name required"
        build_cmd, "build_cmd required"

schema SharedConfig:
    """Kustomize-only resource (ConfigMaps, Secrets)."""
    name: str
    kustomize_path: str
    objects: [str]
    labels: [Label] = ["config"]

    check:
        len(objects) > 0, "must have at least one object"

schema PortForwardResource:
    """kubectl port-forward as local_resource."""
    name: str
    serve_cmd: str
    labels: [Label] = ["port-forward"]
    readiness: ReadinessProbe

schema LocalTask:
    """Arbitrary local command (migrations, codegen, etc.)."""
    name: str
    cmd: str
    labels: [Label] = ["migrations"]
    deps: [str] = []
    resource_deps: [str] = []

# =============================================================================
# Top-level config
# =============================================================================

schema TiltConfig:
    """
    Complete Tilt workspace configuration.

    Produces a validated config that render.k turns into a Tiltfile.
    """
    # Metadata
    repo_type: RepoType = "nx-monorepo"
    languages: [Language] = ["rust"]

    # Base infrastructure kustomize
    base_kustomize?: str

    # Resources
    port_forwards: [PortForwardResource] = []
    local_tasks: [LocalTask] = []
    shared_configs: [SharedConfig] = []
    rust_services: [RustService] = []
    go_services: [GoService] = []
    python_services: [PythonService] = []
    web_apps: [WebApp] = []

    # Cross-validation: no duplicate names
    _all_names: [str] = [
        *[r.name for r in port_forwards]
        *[r.name for r in local_tasks]
        *[r.name for r in shared_configs]
        *[r.name for r in rust_services]
        *[r.name for r in go_services]
        *[r.name for r in python_services]
        *[r.name for r in web_apps]
    ]

    check:
        len(_all_names) == len({n: True for n in _all_names}), "duplicate resource names detected"
