"""
Tekton task builders.
Reusable lambdas for constructing Tekton tasks.
"""

import k8s.apimachinery.pkg.apis.meta.v1 as metav1
import .schemas
import ...schemas.common as c

# =============================================================================
# COMMON TASKS
# =============================================================================
git_clone = lambda -> schemas.TektonTask {
    schemas.TektonTask {
        metadata = metav1.ObjectMeta {name = "git-clone"}
        spec = schemas.TektonTaskSpec {
            description = "Clone the repository"
            workspaces = [
                schemas.TektonWorkspaceDeclaration {name = "source", description = "Workspace to clone into"}
            ]
            params = [
                schemas.TektonParam {name = "repo-url", description = "Git repository URL"}
                schemas.TektonParam {name = "revision", default = "main", description = "Git revision"}
            ]
            steps = [
                schemas.TektonStep {
                    name = "clone"
                    image = c.TEKTON_IMAGES.git
                    script = """\
git clone $(params.repo-url) $(workspaces.source.path)
cd $(workspaces.source.path)
git checkout $(params.revision)
"""
                }
            ]
        }
    }
}

# =============================================================================
# NODE/BUN TASKS
# =============================================================================
install_node_deps = lambda pm: str = "bun" -> schemas.TektonTask {
    _cmd = {
        bun = "bun install"
        npm = "npm ci"
        pnpm = "pnpm install --frozen-lockfile"
        yarn = "yarn install --frozen-lockfile"
    }[pm]
    _image = c.TEKTON_IMAGES.bun if pm == "bun" else c.TEKTON_IMAGES.node

    schemas.TektonTask {
        metadata = metav1.ObjectMeta {name = "install-deps"}
        spec = schemas.TektonTaskSpec {
            description = "Install project dependencies"
            workspaces = [schemas.TektonWorkspaceDeclaration {name = "source"}]
            steps = [
                schemas.TektonStep {
                    name = "install"
                    image = _image
                    workingDir = "$(workspaces.source.path)"
                    script = _cmd
                }
            ]
        }
    }
}

nx_task = lambda {
    target: str,
    pm: str = "bun",
    use_affected: bool = True,
} -> schemas.TektonTask {
    _image = c.TEKTON_IMAGES.bun if pm == "bun" else c.TEKTON_IMAGES.node

    schemas.TektonTask {
        metadata = metav1.ObjectMeta {name = "nx-${target}"}
        spec = schemas.TektonTaskSpec {
            description = "Run nx ${target}"
            workspaces = [schemas.TektonWorkspaceDeclaration {name = "source"}]
            params = [
                schemas.TektonParam {name = "affected", $type = "string", default = str(use_affected).lower()}
            ]
            steps = [
                schemas.TektonStep {
                    name = "nx-${target}"
                    image = _image
                    workingDir = "$(workspaces.source.path)"
                    script = """\
if [ "$(params.affected)" = "true" ]; then
    ${pm} nx affected -t ${target}
else
    ${pm} nx run-many -t ${target}
fi
"""
                }
            ]
        }
    }
}

node_script = lambda name: str, script: str, pm: str = "bun" -> schemas.TektonTask {
    _image = c.TEKTON_IMAGES.bun if pm == "bun" else c.TEKTON_IMAGES.node

    schemas.TektonTask {
        metadata = metav1.ObjectMeta {name = name}
        spec = schemas.TektonTaskSpec {
            description = "Run ${name}"
            workspaces = [schemas.TektonWorkspaceDeclaration {name = "source"}]
            steps = [
                schemas.TektonStep {
                    name = name
                    image = _image
                    workingDir = "$(workspaces.source.path)"
                    script = "${pm} run ${script}"
                }
            ]
        }
    }
}

# =============================================================================
# RUST TASKS
# =============================================================================
cargo_task = lambda cmd: str, args: str = "" -> schemas.TektonTask {
    schemas.TektonTask {
        metadata = metav1.ObjectMeta {name = "cargo-${cmd}"}
        spec = schemas.TektonTaskSpec {
            description = "Run cargo ${cmd}"
            workspaces = [schemas.TektonWorkspaceDeclaration {name = "source"}]
            steps = [
                schemas.TektonStep {
                    name = "cargo-${cmd}"
                    image = c.TEKTON_IMAGES.rust
                    workingDir = "$(workspaces.source.path)"
                    script = "cargo ${cmd}" + (" ${args}" if args else "")
                }
            ]
        }
    }
}

# =============================================================================
# GO TASKS
# =============================================================================
go_task = lambda cmd: str, args: str = "" -> schemas.TektonTask {
    schemas.TektonTask {
        metadata = metav1.ObjectMeta {name = "go-${cmd}"}
        spec = schemas.TektonTaskSpec {
            description = "Run go ${cmd}"
            workspaces = [schemas.TektonWorkspaceDeclaration {name = "source"}]
            steps = [
                schemas.TektonStep {
                    name = "go-${cmd}"
                    image = c.TEKTON_IMAGES.go
                    workingDir = "$(workspaces.source.path)"
                    script = "go ${cmd}" + (" ${args}" if args else "")
                }
            ]
        }
    }
}
