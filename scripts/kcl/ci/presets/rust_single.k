"""
Preset: Rust Single Application
A well-tested CI flow for single Rust applications with cargo.
"""

import ..schemas.config as cfg

# =============================================================================
# PRESET CONFIGURATION
# =============================================================================
schema RustSinglePreset:
    """
    Pre-configured CI for Rust single applications.

    Features:
    - cargo fmt + clippy checks
    - cargo test with all features
    - Release build
    - Optional sccache for faster builds
    - Optional cross-compilation

    Example usage:
        import presets.rust_single as rust
        config = rust.default_config("my-app")
    """
    name: str
    main_branch: str = "main"
    use_sccache: bool = False
    sccache_bucket?: str
    sccache_provider: "gcp" | "aws" = "gcp"
    cargo_features: [str] = []
    clippy: bool = True
    rustfmt: bool = True
    cross_compile: bool = False
    target_triples: [str] = []

# =============================================================================
# PRESET GENERATORS
# =============================================================================
default_config = lambda name: str, use_sccache: bool = False -> cfg.RustAppConfig {
    cfg.RustAppConfig {
        name = name
        main_branch = "main"
        use_sccache = use_sccache
        clippy = True
        rustfmt = True
        use_cargo_cache = True
        cargo_profile = "release"
    }
}

with_sccache_gcp = lambda name: str, bucket: str -> cfg.RustAppConfig {
    cfg.RustAppConfig {
        name = name
        main_branch = "main"
        use_sccache = True
        sccache_bucket = bucket
        sccache_provider = "gcp"
        clippy = True
        rustfmt = True
        # sccache handles this
        use_cargo_cache = False
        cargo_profile = "release"
    }
}

minimal = lambda name: str -> cfg.RustAppConfig {
    """Minimal Rust CI - just build and test, no linting"""
    cfg.RustAppConfig {
        name = name
        main_branch = "main"
        clippy = False
        rustfmt = False
        use_cargo_cache = True
        cargo_profile = "release"
    }
}

library = lambda name: str -> cfg.RustAppConfig {
    """Rust library CI - no release build, focus on testing"""
    cfg.RustAppConfig {
        name = name
        main_branch = "main"
        clippy = True
        rustfmt = True
        use_cargo_cache = True
        cargo_profile = "dev"
        # No build job
        targets = ["lint", "test"]
    }
}
