//! Test fixtures shared by [`crate::option_methods`] and [`crate::result_methods`].
//!
//! Compiled only under `cfg(test)`, so it costs nothing in a real build.

use crate::domain::{Autoscaler, Cluster, NodePool, Registry};

/// Key that is present and fully populated.
pub const PROD: &str = "prod-eu";
/// Key that is present but degenerate: no pools, no autoscaler.
pub const DEV: &str = "dev-eu";
/// Key that is not in the registry at all.
pub const MISSING: &str = "dsa";

/// `prod-eu`: 2 pools, has an autoscaler.
/// `dev-eu`:  0 pools, no autoscaler.
///
/// The three keys give the three cases every combinator has to distinguish:
/// present-and-useful, present-but-empty, and absent.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    r.insert(
        PROD.into(),
        Cluster {
            name: PROD.into(),
            region: "eu".into(),
            pools: vec![
                NodePool {
                    name: "sys".into(),
                    size: 3,
                    spot: false,
                },
                NodePool {
                    name: "batch".into(),
                    size: 10,
                    spot: true,
                },
            ],
            autoscaler: Some(Autoscaler { min: 1, max: 20 }),
        },
    );
    r.insert(
        DEV.into(),
        Cluster {
            name: DEV.into(),
            region: "eu".into(),
            pools: vec![],
            autoscaler: None,
        },
    );
    r
}
