"""
Tiltfile Starlark code generator.

Takes a validated TiltConfig and produces actual Tiltfile content.
"""

import .schema as s

# =============================================================================
# Helpers
# =============================================================================

_HEADER = """\
# =============================================================================
# AUTO-GENERATED Tiltfile — do not edit manually.
# Source: scripts/kcl/tilt/main.k
# Regenerate: kcl run scripts/kcl/tilt/main.k | tee Tiltfile
# =============================================================================
"""

_indent = lambda text: str, level: int = 1 -> str {
    "\n".join(["{}{}".format("    " * level, line) if line.strip() else "" for line in text.split("\n")])
}

_quote = lambda v: str -> str {
    "'{}'".format(v)
}

_quote_list = lambda items: [str] -> str {
    "[{}]".format(", ".join([_quote(i) for i in items]))
}

_port_forward_str = lambda pf: s.PortForwardSpec -> str {
    "'{}:{}'".format(pf.local, pf.remote)
}

# =============================================================================
# Port-forward resources
# =============================================================================

_render_port_forward = lambda pf: s.PortForwardResource -> str {
    """\
local_resource(
    '{}',
    serve_cmd='{}',
    labels={},
    readiness_probe=probe(
        period_secs={},
        exec=exec_action(['sh', '-c', '{}'])
    ),
)
""".format(pf.name, pf.serve_cmd, _quote_list(pf.labels), pf.readiness.period_secs, pf.readiness.exec_cmd)
}

# =============================================================================
# Local tasks
# =============================================================================

_render_local_task = lambda task: s.LocalTask -> str {
    _deps_line = "\n    deps={},".format(_quote_list(task.deps)) if task.deps else ""
    _rdeps_line = "\n    resource_deps={},".format(_quote_list(task.resource_deps)) if task.resource_deps else ""
    """\
local_resource(
    '{}',
    cmd='{}',
    labels={},{}{}\n)\n""".format(task.name, task.cmd, _quote_list(task.labels), _deps_line, _rdeps_line)
}

# =============================================================================
# Shared configs
# =============================================================================

_render_shared_config = lambda sc: s.SharedConfig -> str {
    """\
k8s_yaml(kustomize('{}'))

k8s_resource(
    objects={},
    new_name='{}',
    labels={},
)
""".format(sc.kustomize_path, _quote_list(sc.objects), sc.name, _quote_list(sc.labels))
}

# =============================================================================
# Docker build (shared)
# =============================================================================

_render_docker_build = lambda img: s.Image, only: [str] = [], ignore: [str] = [], live_update: [s.LiveUpdateRule] = [] -> str {
    _args_str = ", ".join(["'{}': '{}'".format(k, v) for k, v in img.build_args])
    _only_str = "\n    only={},".format(_quote_list(only)) if only else ""
    _ignore_str = "\n    ignore={},".format(_quote_list(ignore)) if ignore else ""
    _target_str = "\n    target='{}',".format(img.target) if img.target else ""
    _lu_entries = ["sync('{}', '{}')".format(lu.local_path, lu.container_path) for lu in live_update]
    _lu_str = "\n    live_update=[{}],".format(", ".join(_lu_entries)) if live_update else ""

    """\
docker_build(
    '{}',
    context='{}',
    dockerfile='{}',
    build_args={{{}}},{}{}{}{}\n)\n""".format(img.name, img.context, img.dockerfile, _args_str, _only_str, _ignore_str, _target_str, _lu_str)
}

# =============================================================================
# Rust service
# =============================================================================

_render_rust_service = lambda svc: s.RustService -> str {
    _docker = _render_docker_build(svc.image, svc.image.only, svc.image.ignore, svc.live_update)
    _kustomize = "k8s_yaml(kustomize('{}'))\n".format(svc.kustomize_path)
    _pf = ", ".join([_port_forward_str(p) for p in svc.port_forwards])
    _pf_str = "port_forwards=[{}], ".format(_pf) if svc.port_forwards else ""
    _rdeps_str = "resource_deps={}, ".format(_quote_list(svc.resource_deps)) if svc.resource_deps else ""
    _resource = "k8s_resource('{}', {}{}labels={})\n".format(svc.name, _pf_str, _rdeps_str, _quote_list(svc.labels))

    _docker + "\n" + _kustomize + "\n" + _resource
}

# =============================================================================
# Go service
# =============================================================================

_render_go_service = lambda svc: s.GoService -> str {
    _docker = _render_docker_build(svc.image, svc.image.only, svc.image.ignore, svc.live_update)
    _kustomize = "k8s_yaml(kustomize('{}'))\n".format(svc.kustomize_path)
    _pf = ", ".join([_port_forward_str(p) for p in svc.port_forwards])
    _pf_str = "port_forwards=[{}], ".format(_pf) if svc.port_forwards else ""
    _rdeps_str = "resource_deps={}, ".format(_quote_list(svc.resource_deps)) if svc.resource_deps else ""
    _resource = "k8s_resource('{}', {}{}labels={})\n".format(svc.name, _pf_str, _rdeps_str, _quote_list(svc.labels))

    _docker + "\n" + _kustomize + "\n" + _resource
}

# =============================================================================
# Python service
# =============================================================================

_render_python_service = lambda svc: s.PythonService -> str {
    _docker = _render_docker_build(svc.image, svc.image.only, svc.image.ignore, svc.live_update)
    _kustomize = "k8s_yaml(kustomize('{}'))\n".format(svc.kustomize_path)
    _pf = ", ".join([_port_forward_str(p) for p in svc.port_forwards])
    _pf_str = "port_forwards=[{}], ".format(_pf) if svc.port_forwards else ""
    _rdeps_str = "resource_deps={}, ".format(_quote_list(svc.resource_deps)) if svc.resource_deps else ""
    _resource = "k8s_resource('{}', {}{}labels={})\n".format(svc.name, _pf_str, _rdeps_str, _quote_list(svc.labels))

    _docker + "\n" + _kustomize + "\n" + _resource
}

# =============================================================================
# Web app
# =============================================================================

_render_web_app = lambda app: s.WebApp -> str {
    _build_name = "build-{}".format(app.name)
    _watch_str = _quote_list(app.watch_paths) if app.watch_paths else "[]"
    _rdeps_str = "resource_deps={}, ".format(_quote_list(app.resource_deps)) if app.resource_deps else ""

    _local = """\
local_resource(
    '{}',
    dir='.',
    cmd='{}',
    deps={},
    ignore=['k8s'],
    labels={},
    {}
)
""".format(_build_name, app.build_cmd, _watch_str, _quote_list(app.labels), _rdeps_str.rstrip(", "))

    _docker = _render_docker_build(app.image, [], [], [])
    _kustomize = "k8s_yaml(kustomize('{}'))\n".format(app.kustomize_path)
    _pf = ", ".join([_port_forward_str(p) for p in app.port_forwards])
    _pf_str = "port_forwards=[{}], ".format(_pf) if app.port_forwards else ""
    _resource = "k8s_resource('{}', {}resource_deps=['{}'])\n".format(app.name, _pf_str, _build_name)

    _local + "\n" + _docker + "\n" + _kustomize + "\n" + _resource
}

# =============================================================================
# Section separator
# =============================================================================

_section = lambda title: str -> str {
    """
# =============================================================================
# {}
# =============================================================================
""".format(title)
}

# =============================================================================
# Top-level generator
# =============================================================================

generate = lambda config: s.TiltConfig -> str {
    _parts = [_HEADER]

    # Base kustomize
    if config.base_kustomize:
        _parts += [_section("Base Infrastructure")]
        _parts += ["k8s_yaml(kustomize('{}'))\n".format(config.base_kustomize)]

    # Port forwards
    if config.port_forwards:
        _parts += [_section("Port Forwards")]
        _parts += [_render_port_forward(pf) for pf in config.port_forwards]

    # Local tasks
    if config.local_tasks:
        _parts += [_section("Local Tasks")]
        _parts += [_render_local_task(t) for t in config.local_tasks]

    # Shared configs
    if config.shared_configs:
        _parts += [_section("Shared Config")]
        _parts += [_render_shared_config(sc) for sc in config.shared_configs]

    # Rust services
    if config.rust_services:
        _parts += [_section("Rust Services")]
        _parts += [_render_rust_service(svc) for svc in config.rust_services]

    # Go services
    if config.go_services:
        _parts += [_section("Go Services")]
        _parts += [_render_go_service(svc) for svc in config.go_services]

    # Python services
    if config.python_services:
        _parts += [_section("Python Services")]
        _parts += [_render_python_service(svc) for svc in config.python_services]

    # Web apps
    if config.web_apps:
        _parts += [_section("Web Apps")]
        _parts += [_render_web_app(app) for app in config.web_apps]

    "\n".join(_parts)
}
