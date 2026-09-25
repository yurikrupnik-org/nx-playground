"""
Preset: TanStack Application
A well-tested CI flow for TanStack Router/Query based applications.
"""

import ..schemas.config as cfg

# =============================================================================
# PRESET GENERATORS
# =============================================================================

default_config = lambda name: str, pm: str = "bun" -> cfg.NodeAppConfig {
    """
    Standard TanStack app CI.

    Features:
    - Bun/npm/pnpm/yarn support
    - ESLint/Biome linting
    - Vitest testing
    - Vite build

    Example:
        import presets.tanstack_app as tanstack
        config = tanstack.default_config("my-app")
    """
    cfg.NodeAppConfig {
        name = name
        main_branch = "main"
        package_manager = pm
        framework = "tanstack"
        test_runner = "vitest"
        build_dir = "dist"
    }
}

with_e2e = lambda name: str, pm: str = "bun" -> cfg.NodeAppConfig {
    """TanStack app with Playwright E2E tests"""
    cfg.NodeAppConfig {
        name = name
        main_branch = "main"
        package_manager = pm
        framework = "tanstack"
        test_runner = "playwright"
        e2e = True
        build_dir = "dist"
    }
}

with_ssr = lambda name: str, pm: str = "bun" -> cfg.NodeAppConfig {
    """TanStack Start (SSR) app"""
    cfg.NodeAppConfig {
        name = name
        main_branch = "main"
        package_manager = pm
        framework = "tanstack"
        test_runner = "vitest"
        build_dir = ".output"
    }
}

minimal = lambda name: str -> cfg.NodeAppConfig {
    """Minimal config - task lint and build"""
    cfg.NodeAppConfig {
        name = name
        main_branch = "main"
        package_manager = "bun"
        framework = "tanstack"
        test_runner = "none"
        build_dir = "dist"
        targets = ["lint", "build"]
    }
}
