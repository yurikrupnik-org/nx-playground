"""
GitHub Actions workflow generator.
"""

import github_workflow as gh
import ...schemas.config as cfg
import .steps
import ...schemas.common as c

# =============================================================================
# STEP SEQUENCE BUILDERS
# =============================================================================

_sccache_env_gcp = {
    CARGO_INCREMENTAL = "0"
    RUSTC_WRAPPER = "sccache"
    SCCACHE_GCS_BUCKET = "\${{ vars.SCCACHE_GCS_BUCKET }}"
    SCCACHE_GCS_RW_MODE = "READ_WRITE"
    SCCACHE_GCS_KEY_PREFIX = "sccache"
}

_sccache_env_aws = {
    CARGO_INCREMENTAL = "0"
    RUSTC_WRAPPER = "sccache"
    SCCACHE_BUCKET = "\${{ vars.SCCACHE_BUCKET }}"
    SCCACHE_REGION = "\${{ vars.SCCACHE_REGION }}"
}

# =============================================================================
# ACTION VERSION OVERRIDES
# =============================================================================
# Remap `uses:` pins from gh_config.action_versions (keyed by common.GHA_ACTIONS
# logical name) over the assembled jobs, so no step builder needs the map.

_finalize_jobs = lambda jobs: {str: {str:}}, gh_config: cfg.GitHubConfig -> {str:} {
    _refs = {c.GHA_ACTIONS[k]: gh_config.action_versions[k] for k in gh_config.action_versions if k in c.GHA_ACTIONS}
    {
        name: (job | {"steps" = [(s | {"uses" = _refs[s.uses]}) if ("uses" in s and s.uses in _refs) else s for s in job.steps]})
        for name, job in jobs
    } if _refs else jobs
}

# =============================================================================
# MONOREPO WORKFLOW GENERATOR
# =============================================================================

_monorepo_base_steps = lambda config: cfg.MonorepoConfig -> [{str:}] {
    [
        steps.checkout(0)
        steps.nx_shas() if config.monorepo_tool == "nx" else None
        # sccache setup (before rust tools)
        steps.rustup(config.rust_channel) if config.use_sccache or "rust" in config.languages else None
        steps.gcp_auth() if config.use_sccache and config.sccache_provider == "gcp" else None
        steps.aws_auth() if config.use_sccache and config.sccache_provider == "aws" else None
        steps.sccache() if config.use_sccache else None
        # Tool setup
        steps.mise() if config.use_mise else None
        steps.bun(config.bun_version) if not config.use_mise and config.package_manager == "bun" else None
        steps.node(config.node_version) if not config.use_mise and config.package_manager in ["npm", "pnpm", "yarn"] else None
        steps.go(config.go_version) if not config.use_mise and "go" in config.languages else None
        steps.uv(config.python_version) if not config.use_mise and "python" in config.languages and config.python_manager == "uv" else None
        steps.python(config.python_version) if not config.use_mise and "python" in config.languages and config.python_manager != "uv" else None
        steps.install_deps(config.package_manager)
    ]
}

_monorepo_job = lambda config: cfg.MonorepoConfig, target: str, needs: [str] = [], runs_on: str = "ubuntu-latest" -> {str:} {
    _step_fn = steps.nx_affected if config.use_affected else steps.nx_run_many
    _base = {
        name = target[0].upper() + target[1:]
        "runs-on" = runs_on
        steps = [s for s in _monorepo_base_steps(config) if s] + [
            _step_fn(target, config.package_manager)
        ]
    }
    _base | ({"needs" = needs} if needs else {})
}

generate_monorepo = lambda config: cfg.MonorepoConfig, gh_config: cfg.GitHubConfig = cfg.GitHubConfig {} -> gh.GithubWorkflow {
    _env = {"NX_CLOUD_ACCESS_TOKEN" = "\${{ secrets.NX_CLOUD_ACCESS_TOKEN }}"} if config.nx_cloud else {}
    _sccache_env = _sccache_env_gcp if config.sccache_provider == "gcp" else _sccache_env_aws
    _env = _env | (_sccache_env if config.use_sccache else {})
    _env = _env | config.env

    gh.GithubWorkflow {
        name = "CI"
        on = {
            push = {branches = [config.main_branch]}
            pull_request = {}
        }
        env = _env if _env else Undefined
        permissions = gh_config.permissions
        jobs = _finalize_jobs({
            lint = _monorepo_job(config, "lint", [], gh_config.runs_on)
            test = _monorepo_job(config, "test", [], gh_config.runs_on)
            build = _monorepo_job(config, "build", [], gh_config.runs_on)
        }, gh_config)
    }
}

# =============================================================================
# RUST APP WORKFLOW GENERATOR
# =============================================================================

_rust_base_steps = lambda config: cfg.RustAppConfig -> [{str:}] {
    _components = (["clippy"] if config.clippy else []) + (["rustfmt"] if config.rustfmt else [])
    [
        steps.checkout(0)
        steps.rustup(config.rust_channel, _components)
        steps.gcp_auth() if config.use_sccache and config.sccache_provider == "gcp" else None
        steps.aws_auth() if config.use_sccache and config.sccache_provider == "aws" else None
        steps.sccache() if config.use_sccache else None
        steps.rust_cache() if config.use_cargo_cache and not config.use_sccache else None
    ]
}

generate_rust = lambda config: cfg.RustAppConfig, gh_config: cfg.GitHubConfig = cfg.GitHubConfig {} -> gh.GithubWorkflow {
    _sccache_env = _sccache_env_gcp if config.sccache_provider == "gcp" else _sccache_env_aws
    _env = _sccache_env if config.use_sccache else {}
    _env = _env | config.env
    _features = "--features " + ",".join(config.cargo_features) if config.cargo_features else ""

    _check_steps = [s for s in _rust_base_steps(config) if s]
    if config.rustfmt:
        _check_steps += [steps.cargo("fmt", "--check")]
    if config.clippy:
        _check_steps += [steps.cargo("clippy", "${_features} -- -D warnings")]
    _check_steps += [steps.cargo("check", _features)]

    gh.GithubWorkflow {
        name = "CI"
        on = {
            push = {branches = [config.main_branch]}
            pull_request = {}
        }
        env = _env if _env else Undefined
        permissions = gh_config.permissions
        jobs = _finalize_jobs({
            check = {
                name = "Check"
                "runs-on" = gh_config.runs_on
                steps = _check_steps
            }
            test = {
                name = "Test"
                "runs-on" = gh_config.runs_on
                steps = [s for s in _rust_base_steps(config) if s] + [
                    steps.cargo("test", _features)
                ]
            }
            build = {
                name = "Build"
                "runs-on" = gh_config.runs_on
                needs = ["check", "test"]
                steps = [s for s in _rust_base_steps(config) if s] + [
                    steps.cargo("build", "--release ${_features}")
                ]
            }
        }, gh_config)
    }
}

# =============================================================================
# NODE APP WORKFLOW GENERATOR
# =============================================================================

_node_base_steps = lambda config: cfg.NodeAppConfig -> [{str:}] {
    [
        steps.checkout(0)
        steps.mise() if config.use_mise else None
        steps.bun(config.bun_version) if not config.use_mise and config.package_manager == "bun" else None
        steps.node(config.node_version) if not config.use_mise and config.package_manager != "bun" else None
        steps.install_deps(config.package_manager)
    ]
}

generate_node = lambda config: cfg.NodeAppConfig, gh_config: cfg.GitHubConfig = cfg.GitHubConfig {} -> gh.GithubWorkflow {
    _pm = config.package_manager
    _test_cmd = {
        vitest = "${_pm} run test"
        jest = "${_pm} run test"
        playwright = "${_pm} run test:e2e"
        none = ""
    }[config.test_runner]

    _test_steps = [s for s in _node_base_steps(config) if s]
    if _test_cmd:
        _test_steps += [steps.run_script("Test", _test_cmd)]

    gh.GithubWorkflow {
        name = "CI"
        on = {
            push = {branches = [config.main_branch]}
            pull_request = {}
        }
        env = config.env if config.env else Undefined
        permissions = gh_config.permissions
        jobs = _finalize_jobs({
            lint = {
                name = "Lint"
                "runs-on" = gh_config.runs_on
                steps = [s for s in _node_base_steps(config) if s] + [
                    steps.run_script("Lint", "${_pm} run lint")
                ]
            }
            test = {
                name = "Test"
                "runs-on" = gh_config.runs_on
                steps = _test_steps
            }
            build = {
                name = "Build"
                "runs-on" = gh_config.runs_on
                needs = ["lint", "test"]
                steps = [s for s in _node_base_steps(config) if s] + [
                    steps.run_script("Build", "${_pm} run build")
                ]
            }
        }, gh_config)
    }
}

# =============================================================================
# TAURI APP WORKFLOW GENERATOR
# =============================================================================

generate_tauri = lambda config: cfg.TauriAppConfig, gh_config: cfg.GitHubConfig = cfg.GitHubConfig {} -> gh.GithubWorkflow {
    _pm = config.package_manager
    _os_map = {
        linux = "ubuntu-latest"
        macos = "macos-latest"
        windows = "windows-latest"
    }

    _test_steps = [
        steps.checkout(0)
        steps.bun(config.bun_version) if config.package_manager == "bun" else steps.node(config.node_version)
        steps.rustup()
    ]
    if config.use_cargo_cache:
        _test_steps += [steps.rust_cache()]
    _test_steps += [
        steps.install_deps(config.package_manager)
        steps.run_script("Lint", "${_pm} run lint")
        steps.run_script("Test", "${_pm} run test")
    ]

    _build_steps = [
        steps.checkout(0)
        steps.bun(config.bun_version) if config.package_manager == "bun" else steps.node(config.node_version)
        steps.rustup()
    ]
    if config.use_cargo_cache:
        _build_steps += [steps.rust_cache()]
    _build_steps += [
        steps.install_deps(config.package_manager)
        steps.tauri_build()
    ]

    gh.GithubWorkflow {
        name = "CI"
        on = {
            push = {branches = [config.main_branch]}
            pull_request = {}
        }
        permissions = gh_config.permissions
        jobs = _finalize_jobs({
            test = {
                name = "Test"
                "runs-on" = "ubuntu-latest"
                steps = _test_steps
            }
            build = {
                name = "Build (\${{ matrix.os }})"
                "runs-on" = "\${{ matrix.os }}"
                needs = ["test"]
                strategy = {
                    matrix = {
                        os = [_os_map[t] for t in config.build_targets]
                    }
                }
                steps = _build_steps
            }
        }, gh_config)
    }
}

# =============================================================================
# GO SERVICE WORKFLOW GENERATOR
# =============================================================================

_go_base_steps = lambda config: cfg.GoServiceConfig -> [{str:}] {
    [
        steps.checkout(0)
        steps.go(config.go_version, config.use_go_cache)
    ]
}

generate_go = lambda config: cfg.GoServiceConfig, gh_config: cfg.GitHubConfig = cfg.GitHubConfig {} -> gh.GithubWorkflow {
    _env = {"GOPRIVATE" = config.go_private} if config.go_private else {}
    _env = _env | config.env

    gh.GithubWorkflow {
        name = "CI"
        on = {
            push = {branches = [config.main_branch]}
            pull_request = {}
        }
        env = _env if _env else Undefined
        permissions = gh_config.permissions
        jobs = _finalize_jobs({
            lint = {
                name = "Lint"
                "runs-on" = gh_config.runs_on
                steps = [s for s in _go_base_steps(config) if s] + [
                    steps.run_script("golangci-lint", "go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest && golangci-lint run")
                ]
            }
            test = {
                name = "Test"
                "runs-on" = gh_config.runs_on
                steps = [s for s in _go_base_steps(config) if s] + [
                    steps.go_cmd("test", "-v -race ./...")
                ]
            }
            build = {
                name = "Build"
                "runs-on" = gh_config.runs_on
                needs = ["lint", "test"]
                steps = [s for s in _go_base_steps(config) if s] + [
                    steps.go_cmd("build", config.ldflags or "-v ./...")
                ]
            }
        }, gh_config)
    }
}

# =============================================================================
# EXPORTS
# =============================================================================
# Use generate_monorepo, generate_rust, generate_node, generate_tauri, generate_go
# directly based on your config type.
