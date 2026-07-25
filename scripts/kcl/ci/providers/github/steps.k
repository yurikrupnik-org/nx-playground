"""
GitHub Actions step builders.
Reusable lambdas for constructing workflow steps.
"""

import ...schemas.common as c

# =============================================================================
# CHECKOUT & SETUP
# =============================================================================

checkout = lambda fetch_depth: int = 0 -> {str:} {
    {
        name = "Checkout"
        uses = c.GHA_ACTIONS.checkout
        "with" = {"fetch-depth" = fetch_depth}
    }
}

# =============================================================================
# TOOL SETUP STEPS
# =============================================================================

mise = lambda working_dir: str = "", cache_prefix: str = "mise" -> {str:} {
    {
        name = "Setup mise"
        uses = c.GHA_ACTIONS.mise
        "with" = {
            experimental = "true"
            cache = "true"
            cache_key_prefix = "${cache_prefix}-\${{ runner.os }}"
        } | ({"working_directory" = working_dir} if working_dir else {})
    }
}

bun = lambda version: str = c.DEFAULT_BUN_VERSION -> {str:} {
    {
        name = "Setup Bun"
        uses = c.GHA_ACTIONS.setup_bun
    } | ({"with" = {"bun-version" = version}} if version != c.DEFAULT_BUN_VERSION else {})
}

node = lambda version: str = c.DEFAULT_NODE_VERSION -> {str:} {
    {
        name = "Setup Node.js"
        uses = c.GHA_ACTIONS.setup_node
        "with" = {"node-version" = version}
    }
}

go = lambda version: str = c.DEFAULT_GO_VERSION, cache: bool = True -> {str:} {
    {
        name = "Setup Go"
        uses = c.GHA_ACTIONS.setup_go
        "with" = {
            "go-version" = version
            cache = str(cache).lower()
        }
    }
}

python = lambda version: str = c.DEFAULT_PYTHON_VERSION -> {str:} {
    {
        name = "Setup Python"
        uses = c.GHA_ACTIONS.setup_python
        "with" = {"python-version" = version}
    }
}

uv = lambda python_version: str = "" -> {str:} {
    {
        name = "Setup uv"
        uses = c.GHA_ACTIONS.setup_uv
    } | ({"with" = {"python-version" = python_version}} if python_version else {})
}

rustup = lambda channel: str = "stable", components: [str] = [] -> {str:} {
    _components = ",".join(components) if components else ""
    {
        name = "Install Rust toolchain"
        run = "rustup toolchain install ${channel} --profile minimal" + (" --component ${_components}" if _components else "")
    }
}

rust_cache = lambda -> {str:} {
    {
        name = "Cache Rust dependencies"
        uses = c.GHA_ACTIONS.rust_cache
    }
}

# =============================================================================
# CACHE & AUTH STEPS
# =============================================================================

sccache = lambda -> {str:} {
    {
        name = "Setup sccache"
        uses = c.GHA_ACTIONS.sccache
    }
}

gcp_auth = lambda credentials_secret: str = "GCP_SA_KEY" -> {str:} {
    _prefix = r"${{ secrets."
    _suffix = r" }}"
    _secret_ref = _prefix + credentials_secret + _suffix
    {
        name = "Authenticate to Google Cloud"
        uses = c.GHA_ACTIONS.gcp_auth
        "with" = {"credentials_json" = _secret_ref}
    }
}

aws_auth = lambda role_arn_secret: str = "AWS_ROLE_ARN", region: str = "us-east-1" -> {str:} {
    _prefix = r"${{ secrets."
    _suffix = r" }}"
    _secret_ref = _prefix + role_arn_secret + _suffix
    {
        name = "Configure AWS credentials"
        uses = c.GHA_ACTIONS.aws_auth
        "with" = {
            "role-to-assume" = _secret_ref
            "aws-region" = region
        }
    }
}

# =============================================================================
# PACKAGE MANAGER STEPS
# =============================================================================

install_deps = lambda pm: str = "bun" -> {str:} {
    _cmd = {
        bun = "bun install --frozen-lockfile"
        npm = "npm ci"
        pnpm = "pnpm install --frozen-lockfile"
        yarn = "yarn install --frozen-lockfile"
        pip = "pip install -r requirements.txt"
        poetry = "poetry install"
        uv = "uv sync --frozen"
    }[pm]
    {
        name = "Install dependencies"
        run = _cmd
    }
}

# =============================================================================
# MONOREPO STEPS
# =============================================================================

nx_shas = lambda -> {str:} {
    {
        name = "Set NX SHAs"
        uses = c.GHA_ACTIONS.nx_shas
    }
}

nx_affected = lambda target: str, pm: str = "bun" -> {str:} {
    {
        name = "Run ${target}"
        run = "${pm} nx affected -t ${target}"
    }
}

nx_run_many = lambda target: str, pm: str = "bun" -> {str:} {
    {
        name = "Run ${target} (all)"
        run = "${pm} nx run-many -t ${target}"
    }
}

turbo_run = lambda target: str, pm: str = "bun" -> {str:} {
    {
        name = "Run ${target}"
        run = "${pm} turbo run ${target}"
    }
}

# =============================================================================
# BUILD & TEST STEPS
# =============================================================================

run_script = lambda name: str, script: str -> {str:} {
    {
        name = name
        run = script
    }
}

cargo = lambda cmd: str, args: str = "" -> {str:} {
    {
        name = "Cargo ${cmd}"
        run = "cargo ${cmd}" + (" ${args}" if args else "")
    }
}

go_cmd = lambda cmd: str, args: str = "" -> {str:} {
    {
        name = "Go ${cmd}"
        run = "go ${cmd}" + (" ${args}" if args else "")
    }
}

# =============================================================================
# DOCKER STEPS
# =============================================================================

docker_login = lambda registry: str = "", username_secret: str = "DOCKER_USERNAME", password_secret: str = "DOCKER_PASSWORD" -> {str:} {
    _prefix = r"${{ secrets."
    _suffix = r" }}"
    _username_ref = _prefix + username_secret + _suffix
    _password_ref = _prefix + password_secret + _suffix
    {
        name = "Login to Docker registry"
        uses = c.GHA_ACTIONS.docker_login
        "with" = {
            username = _username_ref
            password = _password_ref
        } | ({"registry" = registry} if registry else {})
    }
}

docker_buildx = lambda -> {str:} {
    {
        name = "Set up Docker Buildx"
        uses = c.GHA_ACTIONS.docker_buildx
    }
}

docker_build_push = lambda image: str, tags: [str] = [], push: bool = True, context: str = "." -> {str:} {
    {
        name = "Build and push Docker image"
        uses = c.GHA_ACTIONS.docker_build_push
        "with" = {
            context = context
            push = str(push).lower()
            tags = "\n".join(tags) if tags else image
        }
    }
}

# =============================================================================
# TAURI STEPS
# =============================================================================

tauri_build = lambda -> {str:} {
    {
        name = "Build Tauri app"
        uses = c.GHA_ACTIONS.tauri_action
    }
}
