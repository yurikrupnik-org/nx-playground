"""
Preset: Go service (polyrepo or monorepo member).
"""

import ..schema as s

go_service = lambda name: str, binary: str, app_path: str = ".", dockerfile: str = "Dockerfile", port_forwards: [s.PortForwardSpec] = [], deps: [str] = [] -> s.GoService {
    s.GoService {
        name = name
        binary_name = binary
        app_path = app_path
        image = s.Image {
            name = name
            dockerfile = dockerfile
            target = "runtime"
            build_args = {BINARY = binary}
            only = ["go.mod", "go.sum", "cmd/", "internal/", "pkg/"]
            ignore = ["**/*.md", "**/.git/", "**/k8s/"]
        }
        kustomize_path = "{}/k8s/kustomize/overlays/dev".format(app_path)
        port_forwards = port_forwards
        resource_deps = deps
    }
}

default_config = lambda name: str, binary: str, port: int = 8080 -> s.TiltConfig {
    s.TiltConfig {
        repo_type = "polyrepo"
        languages = ["go"]
        go_services = [
            go_service(name, binary, port_forwards=[s.PortForwardSpec {local = port, remote = port}])
        ]
    }
}
