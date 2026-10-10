//! Shared domain used by every example in this crate.
//!
//! A cluster registry: some clusters exist, some don't (that's the `Option`),
//! and turning raw config into a cluster can fail in named ways (that's the
//! `Result`). Nothing here is combinator-specific — see [`crate::option_methods`]
//! and [`crate::result_methods`] for those.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct NodePool {
    pub name: String,
    pub size: u32,
    pub spot: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Autoscaler {
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, Clone)]
pub struct Cluster {
    pub name: String,
    pub region: String,
    pub pools: Vec<NodePool>,
    pub autoscaler: Option<Autoscaler>, // not every cluster has one
}

/// The "why" that `Option` cannot carry.
#[derive(Debug, Clone, PartialEq)]
pub enum CfgError {
    Missing(&'static str),
    Parse(&'static str),
    Bounds(u32, u32),
}

/// A second error type, one layer up. Exists so `?` has a `From` conversion to
/// perform — the thing `and_then` on `Result` cannot do for you.
#[derive(Debug, Clone, PartialEq)]
pub enum AppError {
    Cfg(CfgError),
    Io(&'static str),
}

impl From<CfgError> for AppError {
    fn from(e: CfgError) -> Self {
        AppError::Cfg(e)
    }
}

pub type Registry = HashMap<String, Cluster>;

/// `Result`-returning leaf: the thing you chain *from*.
pub fn parse_size(raw: &str) -> Result<u32, CfgError> {
    raw.parse().map_err(|_| CfgError::Parse("size"))
}

/// `Result`-returning leaf: the thing you chain *to*.
pub fn bounded(n: u32) -> Result<u32, CfgError> {
    if (1..=100).contains(&n) {
        Ok(n)
    } else {
        Err(CfgError::Bounds(1, 100))
    }
}

/// A fallible lookup, so `Result` chains have somewhere to start.
pub fn lookup<'a>(reg: &'a Registry, key: &str) -> Result<&'a Cluster, CfgError> {
    reg.get(key).ok_or(CfgError::Missing("cluster"))
}
