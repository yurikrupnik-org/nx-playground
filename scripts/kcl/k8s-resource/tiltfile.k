import .apps
import .schema

_docker_base = "../../../manifests/dockers"
_context = "../../.."

_make_be_tiltfile = lambda cfg: schema.AppConfig -> str {
    _full_name = "${cfg.namespace}-${cfg.name}"
    _app_name = cfg.tilt.build_arg_name or cfg.name
    _deps_str = ", ".join(["'${d}'" for d in cfg.tilt.deps])
    _labels_str = ", ".join(["'${l}'" for l in cfg.tilt.labels])
    _port_forward_line = """
k8s_resource("${_full_name}", port_forwards="${cfg.tilt.port_forward}", labels=[${_labels_str}], resource_deps=[${_deps_str}])""" if cfg.tilt.port_forward else """
k8s_resource("${_full_name}", labels=[${_labels_str}], resource_deps=[${_deps_str}])"""
    """\
docker_build(
  '${cfg.image}',
  build_args={
    'APP_NAME': '${_app_name}',
  },
  context='${_context}',
  dockerfile='${_docker_base}/${cfg.tilt.dockerfile}',
  only=[
    'Cargo.toml',
    'Cargo.lock',
    'apps/',
    'libs/',
    'manifests/dockers/${cfg.tilt.dockerfile}',
  ],
  ignore=[
    'target/',
    '**/*.md',
    '**/.git/',
    '**/k8s/',
    '**/Tiltfile',
    'apps/**/node_modules/',
  ],
  live_update=[
    sync('apps/${cfg.namespace}/${cfg.name}/src', '/app/apps/${cfg.namespace}/${cfg.name}/src'),
  ],
  target='${cfg.tilt.docker_target}',
)

k8s_yaml(kustomize('k8s/kustomize/overlays/dev'))
${_port_forward_line}
"""
}

_make_fe_tiltfile = lambda cfg: schema.AppConfig -> str {
    _full_name = "${cfg.namespace}-${cfg.name}"
    _deps_str = ", ".join(["'${d}'" for d in cfg.tilt.deps])
    _labels_str = ", ".join(["'${l}'" for l in cfg.tilt.labels])
    _port_forward_line = """k8s_resource("${_full_name}", port_forwards="${cfg.tilt.port_forward}", resource_deps=['build-${_full_name}'])""" if cfg.tilt.port_forward else """k8s_resource("${_full_name}", resource_deps=['build-${_full_name}'])"""
    """\
local_resource(
    'build-${_full_name}',
    dir="${_context}",
    cmd='${cfg.tilt.build_cmd}',
    deps=['apps/${cfg.namespace}/${cfg.name}/src', 'apps/${cfg.namespace}/${cfg.name}/project.json', 'apps/${cfg.namespace}/${cfg.name}/vite.config.ts'],
    ignore=["k8s"],
    labels=['bun'],
    resource_deps=[],
)

docker_build(
  '${cfg.image}',
  "${_context}",
  target="${cfg.tilt.docker_target}",
  dockerfile="${_docker_base}/${cfg.tilt.dockerfile}",
  build_args={"DIST_PATH":"apps/${cfg.namespace}/${cfg.name}/dist"},
)

k8s_yaml(kustomize('k8s/kustomize/overlays/dev'))

${_port_forward_line}
"""
}

make_tiltfile = lambda cfg: schema.AppConfig -> str {
    _make_fe_tiltfile(cfg) if cfg.tilt.is_frontend else _make_be_tiltfile(cfg)
}

# Generate all Tiltfiles
tiltfiles = {
    "${a.namespace}-${a.name}": make_tiltfile(a)
    for a in apps.apps
}
