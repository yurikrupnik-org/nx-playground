"""
Preset: Single Rust polyrepo.
For standalone Rust projects (not NX monorepo).
"""

import ..schema as s

rust_service = lambda name: str, pkg: str, dockerfile: str = "Dockerfile", port_forwards: [s.PortForwardSpec] = [], deps: [str] = [] -> s.RustService {
    s.RustService {
        name = name
        package_name = pkg
        app_path = "."
        image = s.Image {
            name = name
            dockerfile = dockerfile
            target = "runtime"
            build_args = {APP_NAME = pkg}
            only = ["Cargo.toml", "Cargo.lock", "src/"]
            ignore = ["target/", "**/*.md", "**/.git/", "**/k8s/"]
        }
        kustomize_path = "k8s/kustomize/overlays/dev"
        port_forwards = port_forwards
        resource_deps = deps
    }
}

default_config = lambda name: str, pkg: str, port: int = 8080 -> s.TiltConfig {
    s.TiltConfig {
        repo_type = "polyrepo"
        languages = ["rust"]
        rust_services = [
            rust_service(name, pkg, port_forwards=[s.PortForwardSpec {local = port, remote = port}])
        ]
    }
}
