"""
Multi-provider CI generation using modular KCL schemas.

This package generates CI configurations for multiple providers (GitHub Actions, Tekton)
from a single configuration source. Users can choose from pre-built presets or create
custom configurations.

Usage (run from the consumer repo root):
    # Generate GitHub workflow from ./manifests/ci/ci-config.yaml
    kcl run scripts/kcl/ci/main.k -S githubWorkflow

    # Generate Tekton resources
    kcl run scripts/kcl/ci/main.k -S tektonResources

    # Point at a different config file
    kcl run scripts/kcl/ci/main.k -D config_file=path/to/ci-config.yaml -S githubWorkflow

Presets available:
    - presets/rust_single.k     - Rust single application
    - presets/tanstack_app.k    - TanStack frontend app
    - presets/tauri_app.k       - Tauri desktop app
    - presets/go_service.k      - Go backend service
    - presets/nx_monorepo.k     - NX monorepo
"""

# Import providers
import .providers.github.main as github
import .providers.tekton.main as tekton

# Config loader (reads the consumer repo's ci-config.yaml)
import .loader

# =============================================================================
# CURRENT REPO CONFIGURATION
# =============================================================================

# The repo's CI shape (languages, package manager, caching, ...) is declared
# in manifests/ci/ci-config.yaml - data, not code. Override the path with
# -D config_file=...
config = loader.load_monorepo(option("config_file", default="manifests/ci/ci-config.yaml"))

# =============================================================================
# GENERATE CI FOR CONFIGURED PROVIDERS
# =============================================================================

# GitHub Actions workflow
githubWorkflow = github.generate_monorepo(config)

# Tekton resources (tasks + pipeline)
tektonResources = tekton.generate_monorepo(config)
# --- Per-repo version overrides (optional) -----------------------------------
# Tool versions (node/bun/go/rust/python) are set on the preset config, e.g.
#   config = nx_preset.nx_playground | {bun_version = "1.2.5", rust_channel = "1.83"}
# GHA action pins and Tekton images are overridden per logical name via the
# provider config (keys match common.GHA_ACTIONS / common.TEKTON_IMAGES):
#   import .schemas.config as cfg
#   githubWorkflow = github.generate_monorepo(config, cfg.GitHubConfig {
#       action_versions = {checkout = "actions/checkout@v5", sccache = "mozilla-actions/sccache-action@v0.0.9"}
#   })
#   tektonResources = tekton.generate_monorepo(config, cfg.TektonConfig {
#       images = {rust = "rust:1.86", git = "alpine/git:v2.47.0"}
#   })
# =============================================================================
# EXAMPLE: USING OTHER PRESETS
# =============================================================================
# Uncomment to generate for different app types:
# --- Rust single app ---
# import .presets.rust_single as rust_preset
# rust_config = rust_preset.default_config("my-rust-app")
# rust_github = github.generate_rust(rust_config)
# rust_tekton = tekton.generate_rust(rust_config)
# --- TanStack app ---
# import .presets.tanstack_app as tanstack_preset
# tanstack_config = tanstack_preset.default_config("my-tanstack-app")
# tanstack_github = github.generate_node(tanstack_config)
# tanstack_tekton = tekton.generate_node(tanstack_config)
# --- Tauri app ---
# import .presets.tauri_app as tauri_preset
# tauri_config = tauri_preset.default_config("my-tauri-app")
# tauri_github = github.generate_tauri(tauri_config)
# --- Go service ---
# import .presets.go_service as go_preset
# go_config = go_preset.default_config("my-go-api")
# go_github = github.generate_go(go_config)
# go_tekton = tekton.generate_go(go_config)
