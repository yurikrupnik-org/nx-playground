"""
Tiltfile generator for the nx-playground monorepo.

Generates a complete Tiltfile from validated KCL config.

Usage:
    # Generate Tiltfile for this repo
    kcl run scripts/kcl/tilt/main.k -E tiltfile | tee Tiltfile

    # Validate only (structured output)
    kcl run scripts/kcl/tilt/main.k -E config

Presets available:
    - presets/nx_monorepo.k   - NX monorepo (Rust + Bun)
    - presets/rust_single.k   - Single Rust polyrepo
    - presets/go_service.k    - Go service
    - presets/python_app.k    - Python service (FastAPI, etc.)
    - presets/bun_app.k       - Bun/Node.js web app
"""

import .schema as s
import .render
import .presets.nx_monorepo as nx

# =============================================================================
# THIS REPO'S CONFIGURATION
# =============================================================================

_REGISTRY = "yurikrupnik"

config = s.TiltConfig {
    repo_type = "nx-monorepo"
    languages = ["rust", "bun"]
    base_kustomize = "manifests/k8s/overlays/dev"

    # =========================================================================
    # Port forwards — infrastructure services
    # =========================================================================
    port_forwards = [
        nx.port_forward("postgres", "kubectl port-forward -n dbs deployment/postgres 5432:5432", 5432)
        nx.port_forward("redis", "kubectl port-forward -n dbs deployment/redis 6379:6379", 6379)
        nx.port_forward("mailhog", "kubectl port-forward -n dbs deployment/mailhog 8025:8025", 8025)
        nx.port_forward("istio-gateway", "kubectl port-forward -n gateway svc/main-gateway-istio 8080:80 8443:443", 8080)
        nx.port_forward("kiali", "kubectl port-forward -n istio-system svc/kiali 20001:20001", 20001)
    ]

    # =========================================================================
    # Local tasks — migrations, codegen
    # =========================================================================
    local_tasks = [
        s.LocalTask {
            name = "schema-configmap"
            cmd = "just gen-schema-configmap"
            deps = ["manifests/db/schema.sql"]
        }
        s.LocalTask {
            name = "db-migrate"
            cmd = 'sqlx migrate run --source manifests/db/migrations --database-url "postgres://myuser:mypassword@localhost:5432/mydatabase?sslmode=disable" && kubectl exec -i -n dbs deployment/postgres -- psql -U myuser -d mydatabase < manifests/db/seed.sql'
            resource_deps = ["postgres"]
            deps = ["manifests/db/migrations", "manifests/db/seed.sql"]
        }
    ]

    # =========================================================================
    # Shared config
    # =========================================================================
    shared_configs = [
        nx.shared_config("zerg-shared-config", "apps/zerg/shared/k8s/kustomize/overlays/dev", ["zerg-shared-config:configmap", "zerg-shared-secrets:secret"])
    ]

    # =========================================================================
    # Rust services
    # =========================================================================
    rust_services = [
        s.RustService {
            name = "zerg-api"
            package_name = "zerg_api"
            app_path = "apps/zerg/api"
            image = s.Image {
                name = "${_REGISTRY}/zerg-api"
                dockerfile = "manifests/dockers/rust.Dockerfile"
                target = "rust"
                build_args = {APP_NAME = "zerg_api"}
                only = ["Cargo.toml", "Cargo.lock", "apps/", "libs/"]
                ignore = ["target/", "**/*.md", "**/.git/", "**/k8s/", "**/Tiltfile", "apps/**/node_modules/"]
            }
            port_forwards = [s.PortForwardSpec {local = 5221, remote = 8080}]
            resource_deps = ["zerg-tasks", "zerg-shared-config"]
        }
        s.RustService {
            name = "zerg-tasks"
            package_name = "zerg_tasks"
            app_path = "apps/zerg/tasks"
            image = s.Image {
                name = "${_REGISTRY}/zerg-tasks"
                dockerfile = "manifests/dockers/rust.Dockerfile"
                target = "rust"
                build_args = {APP_NAME = "zerg_tasks"}
                only = ["Cargo.toml", "Cargo.lock", "apps/", "libs/"]
                ignore = ["target/", "**/*.md", "**/.git/", "**/k8s/", "**/Tiltfile", "apps/**/node_modules/"]
            }
            port_forwards = [s.PortForwardSpec {local = 50051, remote = 50051}]
            resource_deps = ["zerg-shared-config"]
        }
        s.RustService {
            name = "zerg-email-nats"
            package_name = "zerg_email_nats"
            app_path = "apps/zerg/email-nats"
            image = s.Image {
                name = "${_REGISTRY}/zerg-email-nats"
                dockerfile = "manifests/dockers/rust.Dockerfile"
                target = "rust"
                build_args = {APP_NAME = "zerg_email_nats"}
                only = ["Cargo.toml", "Cargo.lock", "apps/", "libs/"]
                ignore = ["target/", "**/*.md", "**/.git/", "**/k8s/", "**/Tiltfile", "apps/**/node_modules/"]
            }
            resource_deps = ["zerg-shared-config"]
        }
    ]

    # =========================================================================
    # Web apps
    # =========================================================================
    web_apps = [
        s.WebApp {
            name = "zerg-web"
            build_cmd = "bun nx run zerg-web:build"
            dist_path = "apps/zerg/web/dist"
            app_path = "apps/zerg/web"
            image = s.Image {
                name = "${_REGISTRY}/zerg-web"
                dockerfile = "manifests/dockers/Dockerfile"
                target = "nginx"
                build_args = {DIST_PATH = "apps/zerg/web/dist"}
            }
            port_forwards = [s.PortForwardSpec {local = 5206, remote = 8080}]
            watch_paths = ["apps/zerg/web/src", "apps/zerg/web/project.json", "apps/zerg/web/vite.config.ts"]
        }
    ]
}

# =============================================================================
# GENERATE TILTFILE
# =============================================================================

tiltfile = render.generate(config)
