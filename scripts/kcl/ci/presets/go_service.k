"""
Preset: Go Backend Service
A well-tested CI flow for Go backend services.
"""

import ..schemas.config as cfg

# =============================================================================
# PRESET GENERATORS
# =============================================================================
default_config = lambda name: str -> cfg.GoServiceConfig {
    """
    Standard Go service CI.

    Features:
    - golangci-lint for linting
    - go test with race detection
    - go build with version info

    Example:
        import presets.go_service as go
        config = go.default_config("my-api")
    """
    cfg.GoServiceConfig {
        name = name
        main_branch = "main"
        go_version = "1.23"
        use_go_cache = True
    }
}

with_docker = lambda name: str, registry: str -> cfg.GoServiceConfig {
    """Go service with Docker build and push"""
    cfg.GoServiceConfig {
        name = name
        main_branch = "main"
        go_version = "1.23"
        use_go_cache = True
        build_docker = True
        docker_registry = registry
    }
}

with_private_modules = lambda name: str, go_private: str -> cfg.GoServiceConfig {
    """Go service with private module support"""
    cfg.GoServiceConfig {
        name = name
        main_branch = "main"
        go_version = "1.23"
        use_go_cache = True
        go_private = go_private
    }
}

minimal = lambda name: str -> cfg.GoServiceConfig {
    """Minimal Go CI - test and build only"""
    cfg.GoServiceConfig {
        name = name
        main_branch = "main"
        go_version = "1.23"
        use_go_cache = True
        targets = ["test", "build"]
    }
}

microservice = lambda name: str, registry: str -> cfg.GoServiceConfig {
    """Go microservice with optimized build flags"""
    cfg.GoServiceConfig {
        name = name
        main_branch = "main"
        go_version = "1.23"
        use_go_cache = True
        ldflags = "-s -w -X main.version=\${{ github.sha }}"
        build_docker = True
        docker_registry = registry
    }
}
