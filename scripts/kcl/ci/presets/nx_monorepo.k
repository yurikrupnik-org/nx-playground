"""
Preset: NX Monorepo
A well-tested CI flow for NX-based monorepos.
"""

import ..schemas.config as cfg
import ..schemas.common as c

# =============================================================================
# PRESET GENERATORS
# =============================================================================

default_config = lambda name: str, pm: str = "bun", languages: [c.Language] = ["typescript"] -> cfg.MonorepoConfig {
    """
    Standard NX monorepo CI.

    Features:
    - NX affected commands for efficient CI
    - NX Cloud integration
    - Multi-language support
    - Package manager flexibility

    Example:
        import presets.nx_monorepo as nx
        config = nx.default_config("my-monorepo")
    """
    cfg.MonorepoConfig {
        name = name
        main_branch = "main"
        monorepo_tool = "nx"
        package_manager = pm
        languages = languages
        nx_cloud = True
        use_affected = True

        #    main_branch = "main"
         #   monorepo_tool = "nx"
          #  package_manager = "bun"
          #  languages = ["typescript", "rust", "kcl"]
          #  nx_cloud = True
          #  use_affected = True
          #  use_mise = False  # Use targeted setup actions instead
          #  use_sccache = True
          #  sccache_provider = "gcp"
    }
}

with_rust = lambda name: str, pm: str = "bun", use_sccache: bool = False -> cfg.MonorepoConfig {
    """NX monorepo with Rust projects"""
    cfg.MonorepoConfig {
        name = name
        main_branch = "main"
        monorepo_tool = "nx"
        package_manager = pm
        languages = ["typescript", "rust"]
        nx_cloud = True
        use_affected = True
        use_sccache = use_sccache
    }
}

with_go = lambda name: str, pm: str = "bun" -> cfg.MonorepoConfig {
    """NX monorepo with Go projects"""
    cfg.MonorepoConfig {
        name = name
        main_branch = "main"
        monorepo_tool = "nx"
        package_manager = pm
        languages = ["typescript", "go"]
        nx_cloud = True
        use_affected = True
    }
}

full_stack = lambda name: str, pm: str = "bun" -> cfg.MonorepoConfig {
    """Full-stack NX monorepo (TS + Rust + Go)"""
    cfg.MonorepoConfig {
        name = name
        main_branch = "main"
        monorepo_tool = "nx"
        package_manager = pm
        languages = ["typescript", "rust", "go", "python"]
        nx_cloud = True
        use_affected = True
        use_sccache = True
    }
}

without_cloud = lambda name: str, pm: str = "bun", languages: [c.Language] = ["typescript"] -> cfg.MonorepoConfig {
    """NX monorepo without NX Cloud"""
    cfg.MonorepoConfig {
        name = name
        main_branch = "main"
        monorepo_tool = "nx"
        package_manager = pm
        languages = languages
        nx_cloud = False
        use_affected = True
    }
}

run_all = lambda name: str, pm: str = "bun", languages: [c.Language] = ["typescript"] -> cfg.MonorepoConfig {
    """NX monorepo running all projects (not affected)"""
    cfg.MonorepoConfig {
        name = name
        main_branch = "main"
        monorepo_tool = "nx"
        package_manager = pm
        languages = languages
        nx_cloud = True
        use_affected = False
    }
}

# =============================================================================
# THIS REPO'S CONFIG
# =============================================================================

nx_playground = cfg.MonorepoConfig {
    name = "nx-playground"
    main_branch = "main"
    monorepo_tool = "nx"
    package_manager = "bun"
    languages = ["typescript", "rust", "kcl"]
    nx_cloud = True
    use_affected = True
    use_mise = False  # Use targeted setup actions instead
    use_sccache = True
    sccache_provider = "gcp"
}
