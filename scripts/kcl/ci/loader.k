"""
Consumer config loader.

Repos consuming this package declare their CI shape in a plain data file
(manifests/ci/ci-config.yaml) instead of writing KCL. The file is decoded and
validated against the typed schemas here - unknown languages, package managers,
etc. fail generation with a schema error.

Example manifests/ci/ci-config.yaml:
    name: my-monorepo
    languages: [typescript, python, go]   # controls which toolchains CI sets up
    package_manager: bun
    python_manager: uv
    nx_cloud: true
    use_sccache: false
"""

import file
import yaml
import .schemas.config as cfg

# Load a monorepo config from a YAML file (path relative to `kcl run` cwd).
load_monorepo = lambda path: str = "manifests/ci/ci-config.yaml" -> cfg.MonorepoConfig {
    cfg.MonorepoConfig {**yaml.decode(file.read(path))}
}
