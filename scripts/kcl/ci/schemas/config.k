"""
Configuration schemas for CI generation.
"""
#type Language = "typescript" | "rust" | "go" | "python" | "java" | "kcl"

import .common as c

# =============================================================================
# BASE CONFIGURATION
# =============================================================================

schema BaseConfig:
    """Base configuration shared by all app types"""
    name: str
    main_branch: str = "main"

    # Tool setup options
    use_mise: bool = False
    node_version: str = c.DEFAULT_NODE_VERSION
    go_version: str = c.DEFAULT_GO_VERSION
    rust_channel: str = c.DEFAULT_RUST_CHANNEL
    python_version: str = c.DEFAULT_PYTHON_VERSION

    # Cache options
    use_sccache: bool = False
    sccache_bucket?: str
    sccache_provider: "gcp" | "aws" | "s3" = "gcp"

    # CI targets to run
    targets: [str] = ["lint", "test", "build", "container"]

    # Extra environment variables
    env: {str: str} = {}

    # Extra secrets to inject
    secrets: [str] = []

# =============================================================================
# MONOREPO CONFIGURATION
# =============================================================================

schema MonorepoConfig(BaseConfig):
    """Configuration for monorepo projects"""
    monorepo_tool: c.MonorepoTool = "nx"
    package_manager: c.PackageManager = "bun"
    languages: [c.Language] = ["typescript"]

    # NX specific
    nx_cloud: bool = True
    use_affected: bool = True

# =============================================================================
# SINGLE APP CONFIGURATIONS
# =============================================================================

schema RustAppConfig(BaseConfig):
    """Configuration for single Rust applications"""
    cargo_features: [str] = []
    cargo_profile: "dev" | "release" = "release"
    use_cargo_cache: bool = True
    clippy: bool = True
    rustfmt: bool = True

    # Binary output
    binary_name?: str
    target_triple?: str  # e.g., "x86_64-unknown-linux-gnu"

schema NodeAppConfig(BaseConfig):
    """Configuration for Node.js applications (TanStack, etc.)"""
    package_manager: c.PackageManager = "bun"
    framework: "tanstack" | "react" | "vue" | "svelte" | "solid" | "next" | "nuxt" | "vite" = "vite"

    # Test runner
    test_runner: "vitest" | "jest" | "playwright" | "none" = "vitest"
    e2e: bool = False

    # Build output
    build_dir: str = "dist"

schema TauriAppConfig(BaseConfig):
    """Configuration for Tauri applications"""
    package_manager: c.PackageManager = "bun"
    frontend_framework: "tanstack" | "react" | "vue" | "svelte" | "solid" = "react"

    # Tauri specific
    tauri_version: str = "v2"
    build_targets: [str] = ["linux", "macos", "windows"]
    sign_macos: bool = False
    sign_windows: bool = False

    # Rust backend
    use_cargo_cache: bool = True

schema GoServiceConfig(BaseConfig):
    """Configuration for Go backend services"""
    go_private?: str  # GOPRIVATE env var
    use_go_cache: bool = True

    # Build options
    binary_name?: str
    ldflags?: str

    # Docker
    build_docker: bool = False
    docker_registry?: str

schema PythonAppConfig(BaseConfig):
    """Configuration for Python applications"""
    package_manager: "pip" | "poetry" | "uv" = "uv"

    # Test/lint tools
    pytest: bool = True
    ruff: bool = True
    mypy: bool = False

    # Build
    build_wheel: bool = True

# =============================================================================
# CI GENERATION CONFIG
# =============================================================================

schema CIConfig:
    """Top-level CI generation configuration"""
    # Which providers to generate for
    providers: [c.CIProvider] = ["github"]

    # App configuration (one of the above)
    app: MonorepoConfig | RustAppConfig | NodeAppConfig | TauriAppConfig | GoServiceConfig | PythonAppConfig

    # Provider-specific overrides
    github?: GitHubConfig
    tekton?: TektonConfig

schema GitHubConfig:
    """GitHub Actions specific configuration"""
    runs_on: str = "ubuntu-latest"
    concurrency_group?: str
    cancel_in_progress: bool = True

    # Matrix builds
    matrix?: {str: [str]}

    # Permissions
    permissions: {str: str} = {
        actions = "read"
        contents = "read"
    }

schema TektonConfig:
    """Tekton specific configuration"""
    namespace: str = "default"
    workspace_size: str = "1Gi"
    service_account?: str
