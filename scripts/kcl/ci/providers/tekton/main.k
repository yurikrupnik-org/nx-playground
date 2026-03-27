"""
Tekton pipeline generator.
"""
import k8s.apimachinery.pkg.apis.meta.v1 as metav1
import .schemas
import .tasks
import ...schemas.config as cfg

# =============================================================================
# PIPELINE BUILDERS
# =============================================================================
_pipeline_task = lambda name: str, task_name: str, run_after: [str] = [] -> schemas.TektonPipelineTask {
    schemas.TektonPipelineTask {
        name = name
        taskRef = schemas.TektonTaskRef {name = task_name}
        runAfter = run_after if run_after else Undefined
        workspaces = [
            schemas.TektonPipelineTaskWorkspace {name = "source", workspace = "shared-workspace"}
        ]
    }
}

_clone_task = lambda -> schemas.TektonPipelineTask {
    schemas.TektonPipelineTask {
        name = "clone"
        taskRef = schemas.TektonTaskRef {name = "git-clone"}
        params = [
            schemas.TektonPipelineTaskParam {name = "repo-url", value = "$(params.repo-url)"}
            schemas.TektonPipelineTaskParam {name = "revision", value = "$(params.revision)"}
        ]
        workspaces = [
            schemas.TektonPipelineTaskWorkspace {name = "source", workspace = "shared-workspace"}
        ]
    }
}
# =============================================================================
# MONOREPO PIPELINE GENERATOR
# =============================================================================
generate_monorepo = lambda config: cfg.MonorepoConfig, tekton_config: cfg.TektonConfig = cfg.TektonConfig {} -> [any] {
    _pm = config.package_manager
    _tasks = [
        tasks.git_clone()
        tasks.install_node_deps(_pm)
        tasks.nx_task("lint", _pm, config.use_affected)
        tasks.nx_task("test", _pm, config.use_affected)
        tasks.nx_task("build", _pm, config.use_affected)
        tasks.nx_task("container", _pm, config.use_affected)
    ]

    _pipeline = schemas.TektonPipeline {
        metadata = metav1.ObjectMeta {
            name = "${config.name}-ci"
            namespace = tekton_config.namespace if tekton_config.namespace != "default" else Undefined
        }
        spec = schemas.TektonPipelineSpec {
            description = "CI pipeline for ${config.name}"
            params = [
                schemas.TektonParam {name = "repo-url", description = "Git repository URL"}
                schemas.TektonParam {name = "revision", default = config.main_branch}
            ]
            workspaces = [schemas.TektonPipelineWorkspace {name = "shared-workspace"}]
            tasks = [
                _clone_task()
                _pipeline_task("install", "install-deps", ["clone"])
                _pipeline_task("lint", "nx-lint", ["install"])
                _pipeline_task("test", "nx-test", ["install"])
                _pipeline_task("build", "nx-build", ["lint", "test"])
            ]
        }
    }

    _tasks + [_pipeline]
}
# =============================================================================
# RUST APP PIPELINE GENERATOR
# =============================================================================
generate_rust = lambda config: cfg.RustAppConfig, tekton_config: cfg.TektonConfig = cfg.TektonConfig {} -> [any] {
    _features = "--features " + ",".join(config.cargo_features) if config.cargo_features else ""
    _tasks = [
        tasks.git_clone()
        tasks.cargo_task("fmt", "--check") if config.rustfmt else None
        tasks.cargo_task("clippy", "${_features} -- -D warnings") if config.clippy else None
        tasks.cargo_task("test", _features)
        tasks.cargo_task("build", "--release ${_features}")
    ]

    _pipeline_tasks = [_clone_task()]
    if config.rustfmt:
        _pipeline_tasks += [_pipeline_task("fmt", "cargo-fmt", ["clone"])]

    if config.clippy:
        _after = ["fmt"] if config.rustfmt else ["clone"]
        _pipeline_tasks += [_pipeline_task("clippy", "cargo-clippy", _after)]

    _test_after = ["clippy"] if config.clippy else (["fmt"] if config.rustfmt else ["clone"])
    _pipeline_tasks += [
        _pipeline_task("test", "cargo-test", _test_after)
        _pipeline_task("build", "cargo-build", ["test"])
    ]

    _pipeline = schemas.TektonPipeline {
        metadata = metav1.ObjectMeta {
            name = "${config.name}-ci"
            namespace = tekton_config.namespace if tekton_config.namespace != "default" else Undefined
        }
        spec = schemas.TektonPipelineSpec {
            description = "CI pipeline for ${config.name}"
            params = [
                schemas.TektonParam {name = "repo-url", description = "Git repository URL"}
                schemas.TektonParam {name = "revision", default = config.main_branch}
            ]
            workspaces = [schemas.TektonPipelineWorkspace {name = "shared-workspace"}]
            tasks = _pipeline_tasks
        }
    }

    [t for t in _tasks if t] + [_pipeline]
}
# =============================================================================
# NODE APP PIPELINE GENERATOR
# =============================================================================
generate_node = lambda config: cfg.NodeAppConfig, tekton_config: cfg.TektonConfig = cfg.TektonConfig {} -> [any] {
    _pm = config.package_manager
    _tasks = [
        tasks.git_clone()
        tasks.install_node_deps(_pm)
        tasks.node_script("lint", "lint", _pm)
        tasks.node_script("test", "test", _pm) if config.test_runner != "none" else None
        tasks.node_script("build", "build", _pm)
    ]

    _pipeline_tasks = [
        _clone_task()
        _pipeline_task("install", "install-deps", ["clone"])
        _pipeline_task("lint", "lint", ["install"])
    ]
    if config.test_runner != "none":
        _pipeline_tasks += [_pipeline_task("test", "test", ["install"])]
        _pipeline_tasks += [_pipeline_task("build", "build", ["lint", "test"])]
    else:
        _pipeline_tasks += [_pipeline_task("build", "build", ["lint"])]
    _pipeline = schemas.TektonPipeline {
        metadata = metav1.ObjectMeta {
            name = "${config.name}-ci"
            namespace = tekton_config.namespace if tekton_config.namespace != "default" else Undefined
        }
        spec = schemas.TektonPipelineSpec {
            description = "CI pipeline for ${config.name}"
            params = [
                schemas.TektonParam {name = "repo-url", description = "Git repository URL"}
                schemas.TektonParam {name = "revision", default = config.main_branch}
            ]
            workspaces = [schemas.TektonPipelineWorkspace {name = "shared-workspace"}]
            tasks = _pipeline_tasks
        }
    }

    [t for t in _tasks if t] + [_pipeline]
}
# =============================================================================
# GO SERVICE PIPELINE GENERATOR
# =============================================================================
generate_go = lambda config: cfg.GoServiceConfig, tekton_config: cfg.TektonConfig = cfg.TektonConfig {} -> [any] {
    _tasks = [
        tasks.git_clone()
        tasks.go_task("test", "-v -race ./...")
        tasks.go_task("build", config.ldflags or "-v ./...")
    ]

    _pipeline = schemas.TektonPipeline {
        metadata = metav1.ObjectMeta {
            name = "${config.name}-ci"
            namespace = tekton_config.namespace if tekton_config.namespace != "default" else Undefined
        }
        spec = schemas.TektonPipelineSpec {
            description = "CI pipeline for ${config.name}"
            params = [
                schemas.TektonParam {name = "repo-url", description = "Git repository URL"}
                schemas.TektonParam {name = "revision", default = config.main_branch}
            ]
            workspaces = [schemas.TektonPipelineWorkspace {name = "shared-workspace"}]
            tasks = [
                _clone_task()
                _pipeline_task("test", "go-test", ["clone"])
                _pipeline_task("build", "go-build", ["test"])
            ]
        }
    }

    _tasks + [_pipeline]
}
# =============================================================================
# EXPORTS
# =============================================================================
# Use generate_monorepo, generate_rust, generate_node, generate_go
# directly based on your config type.
