"""
Preset: Tauri Application
A well-tested CI flow for Tauri desktop applications.
"""

import ..schemas.config as cfg

# =============================================================================
# PRESET GENERATORS
# =============================================================================

default_config = lambda name: str, pm: str = "bun" -> cfg.TauriAppConfig {
    """
    Standard Tauri v2 app CI.

    Features:
    - Cross-platform builds (Linux, macOS, Windows)
    - Rust backend with cargo cache
    - Frontend testing before build
    - tauri-action for releases

    Example:
        import presets.tauri_app as tauri
        config = tauri.default_config("my-desktop-app")
    """
    cfg.TauriAppConfig {
        name = name
        main_branch = "main"
        package_manager = pm
        frontend_framework = "vue"
        tauri_version = "v2"
        build_targets = ["linux", "macos", "windows"]
        use_cargo_cache = True
    }
}

linux_only = lambda name: str, pm: str = "bun" -> cfg.TauriAppConfig {
    """Tauri app targeting Linux only"""
    cfg.TauriAppConfig {
        name = name
        main_branch = "main"
        package_manager = pm
        frontend_framework = "react"
        tauri_version = "v2"
        build_targets = ["linux"]
        use_cargo_cache = True
    }
}

with_signing = lambda name: str, pm: str = "bun" -> cfg.TauriAppConfig {
    """Tauri app with code signing enabled"""
    cfg.TauriAppConfig {
        name = name
        main_branch = "main"
        package_manager = pm
        frontend_framework = "react"
        tauri_version = "v2"
        build_targets = ["linux", "macos", "windows"]
        sign_macos = True
        sign_windows = True
        use_cargo_cache = True
    }
}

with_tanstack = lambda name: str, pm: str = "bun" -> cfg.TauriAppConfig {
    """Tauri app with TanStack frontend"""
    cfg.TauriAppConfig {
        name = name
        main_branch = "main"
        package_manager = pm
        frontend_framework = "tanstack"
        tauri_version = "v2"
        build_targets = ["linux", "macos", "windows"]
        use_cargo_cache = True
    }
}

with_svelte = lambda name: str, pm: str = "bun" -> cfg.TauriAppConfig {
    """Tauri app with Svelte frontend"""
    cfg.TauriAppConfig {
        name = name
        main_branch = "main"
        package_manager = pm
        frontend_framework = "svelte"
        tauri_version = "v2"
        build_targets = ["linux", "macos", "windows"]
        use_cargo_cache = True
    }
}
