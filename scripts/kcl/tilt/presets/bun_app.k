"""
Preset: Bun/Node.js web app (polyrepo or monorepo member).
"""

import ..schema as s

web_app = lambda name: str, app_path: str = ".", pm: str = "bun", dockerfile: str = "Dockerfile", port_forwards: [s.PortForwardSpec] = [] -> s.WebApp {
    s.WebApp {
        name = name
        build_cmd = "{} run build".format(pm)
        dist_path = "{}/dist".format(app_path) if app_path != "." else "dist"
        app_path = app_path
        image = s.Image {
            name = name
            dockerfile = dockerfile
            target = "nginx"
            build_args = {DIST_PATH = "{}/dist".format(app_path) if app_path != "." else "dist"}
        }
        kustomize_path = "{}/k8s/kustomize/overlays/dev".format(app_path)
        port_forwards = port_forwards
        watch_paths = ["{}/src".format(app_path) if app_path != "." else "src"]
        labels = ["frontend"]
    }
}

default_config = lambda name: str, pm: str = "bun", port: int = 5173 -> s.TiltConfig {
    s.TiltConfig {
        repo_type = "polyrepo"
        languages = ["bun"]
        web_apps = [
            web_app(name, pm=pm, port_forwards=[s.PortForwardSpec {local = port, remote = 8080}])
        ]
    }
}
