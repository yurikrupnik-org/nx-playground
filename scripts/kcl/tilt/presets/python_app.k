"""
Preset: Python service (FastAPI, Django, Flask, etc.).
"""

import ..schema as s

python_service = lambda name: str, app_path: str = ".", dockerfile: str = "Dockerfile", port_forwards: [s.PortForwardSpec] = [], deps: [str] = [] -> s.PythonService {
    s.PythonService {
        name = name
        app_path = app_path
        image = s.Image {
            name = name
            dockerfile = dockerfile
            target = "runtime"
            only = ["pyproject.toml", "requirements.txt", "src/", "app/"]
            ignore = ["**/__pycache__/", ".venv/", "**/*.md", "**/.git/", "**/k8s/"]
        }
        kustomize_path = "{}/k8s/kustomize/overlays/dev".format(app_path)
        port_forwards = port_forwards
        resource_deps = deps
        live_update = [s.LiveUpdateRule {
            local_path = "{}/src".format(app_path) if app_path != "." else "src"
            container_path = "/app/src"
        }]
    }
}

default_config = lambda name: str, port: int = 8000 -> s.TiltConfig {
    s.TiltConfig {
        repo_type = "polyrepo"
        languages = ["python"]
        python_services = [
            python_service(name, port_forwards=[s.PortForwardSpec {local = port, remote = port}])
        ]
    }
}
