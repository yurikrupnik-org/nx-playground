"""
Common types and enums for CI configuration.
"""

# =============================================================================
# TYPE DEFINITIONS
# =============================================================================

type Language = "typescript" | "rust" | "go" | "python" | "java" | "kcl"
type PackageManager = "bun" | "npm" | "pnpm" | "yarn" | "cargo" | "go" | "pip" | "poetry" | "uv"
type MonorepoTool = "nx" | "turborepo" | "moonrepo" | "cargo-workspace" | "none"
type CIProvider = "github" | "tekton" | "gitlab" | "circleci"
type AppType = "rust-single" | "tanstack-app" | "tauri-app" | "node-fe" | "go-service" | "nx-monorepo" | "custom"

# =============================================================================
# CONSTANTS
# =============================================================================

# Default versions for tools
DEFAULT_NODE_VERSION = "22"
DEFAULT_BUN_VERSION = "1"
DEFAULT_GO_VERSION = "1.23"
DEFAULT_RUST_CHANNEL = "stable"
DEFAULT_PYTHON_VERSION = "3.12"

# Default images for Tekton
TEKTON_IMAGES = {
    node = "node:22-slim"
    bun = "oven/bun:1"
    go = "golang:1.23"
    rust = "rust:1.83"
    python = "python:3.12-slim"
    uv = "ghcr.io/astral-sh/uv:python3.12-bookworm-slim"
    git = "alpine/git:latest"
}

# GitHub Actions versions
GHA_ACTIONS = {
    checkout = "actions/checkout@v7"
    setup_node = "actions/setup-node@v4"
    setup_bun = "oven/setup-bun@v2"
    setup_go = "actions/setup-go@v5"
    setup_python = "actions/setup-python@v5"
    setup_uv = "astral-sh/setup-uv@v9"
    mise = "jdx/mise-action@v2"
    nx_shas = "nrwl/nx-set-shas@v5"
    sccache = "mozilla-actions/sccache-action@v0.0.10"
    gcp_auth = "google-github-actions/auth@v3"
    aws_auth = "aws-actions/configure-aws-credentials@v4"
    docker_login = "docker/login-action@v3"
    docker_buildx = "docker/setup-buildx-action@v3"
    docker_build_push = "docker/build-push-action@v6"
    tauri_action = "tauri-apps/tauri-action@v0"
    rust_cache = "Swatinem/rust-cache@v2"
}
