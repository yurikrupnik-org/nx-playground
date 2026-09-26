//! `scope:` tag for every graph node the plugin touches — the declared
//! ownership map `task boundaries` (`tools/nx/check-boundaries.ts`) enforces.
//! Mirrors `tools/nx/scope-tags.ts`.
//!
//! The scope is DECLARED, never derived from the dependency graph: a tag
//! computed from who-depends-on-whom would follow every new edge, and a gate
//! keyed on it could never fail. The path alone is not enough either, because
//! two facts cut across the directory layout:
//!
//! - `apps/zerg/tasks` is an extracted service behind an authenticated gRPC
//!   boundary (docs/adr-tasks-service-boundary.md). Its domain crate is private
//!   to it, so the service and its domain get their own scope, `tasks`,
//!   distinct from the `zerg` vertical that hosts them on disk. The wire
//!   contract (`libs/contracts/tasks`) stays `shared`: being consumable from
//!   outside the boundary is its whole job.
//! - `libs/domains/cloud_resources` is consumed by both `zerg_api` and
//!   `terran_api`, so it is honestly `shared` — the undecided bounded-context
//!   seam in docs/architecture-review-todo.md Issue 5. Retag it here (and in
//!   the TS twin) when that call is made.
//!
//! Everything else follows the layout: `apps/<vertical>/**` belongs to its
//! vertical and every lib is `shared` — domains are composed at the app layer.

/// Exact project roots whose scope departs from the path-derived default.
const SCOPE_OVERRIDES: &[(&str, &str)] = &[
    ("apps/zerg/tasks", "tasks"),
    ("libs/domains/tasks", "tasks"),
    ("libs/domains/todo", "todo"),
    ("libs/domains/projects", "zerg"),
    ("libs/domains/users", "zerg"),
    ("libs/domains/vector", "zerg"),
    ("libs/domains/cloud_resources", "shared"),
];

/// `scope:<name>` for a project root, e.g. `apps/todo/worker` -> `scope:todo`.
pub fn scope_tag(dir: &str) -> String {
    if let Some((_, scope)) = SCOPE_OVERRIDES.iter().find(|(root, _)| *root == dir) {
        return format!("scope:{scope}");
    }
    let mut parts = dir.split('/');
    match (parts.next(), parts.next()) {
        (Some("apps"), Some(vertical)) if !vertical.is_empty() => format!("scope:{vertical}"),
        _ => "scope:shared".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::scope_tag;

    #[test]
    fn declared_overrides_beat_the_path() {
        // Hosted under apps/zerg, but behind its own service boundary.
        assert_eq!(scope_tag("apps/zerg/tasks"), "scope:tasks");
        // A lib owned by a vertical rather than shared.
        assert_eq!(scope_tag("libs/domains/users"), "scope:zerg");
        // Only the exact root is overridden, not what lives below it.
        assert_eq!(scope_tag("apps/zerg/tasks/sub"), "scope:zerg");
    }

    #[test]
    fn the_path_decides_everything_else() {
        assert_eq!(scope_tag("apps/todo/worker"), "scope:todo");
        assert_eq!(scope_tag("libs/core/config"), "scope:shared");
        // `apps` alone names no vertical.
        assert_eq!(scope_tag("apps"), "scope:shared");
        assert_eq!(scope_tag("."), "scope:shared");
    }
}
