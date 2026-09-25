//! The Rust port of `tools/nx/plugin.ts` — this repo's inference, run by
//! butler with no node process.
//!
//! The TypeScript plugin stays: nx loads it while computing its own graph,
//! and it must not wait on a cargo build (see the header of
//! `tools/nx/butler-config.ts`). Two implementations of one rule set drift, so
//! `butler graph verify` diffs this layer's result against an `nx graph --file`
//! dump and fails on any disagreement — the same arrangement `container
//! verify` used to cover for the container targets alone.
//!
//! This file mirrors `createNodesV2`/`createDependencies` in `plugin.ts`
//! line for line; the per-module rules live beside it (`crates.rs` for
//! `rust-targets.ts`/`openapi-targets.ts`/`polyglot-targets.ts`, `apps.rs` for
//! `container-targets.ts`/`k8s-targets.ts`/`tilt-targets.ts`, `scope.rs` for
//! `scope-tags.ts`).

use std::collections::{BTreeMap, BTreeSet};

use eyre::{Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};

use super::{Contribution, Ctx, Dependency, Layer, Node, ProjectRoots};

pub mod apps;
pub mod crates;
pub mod scope;

/// `plugin.ts` `MARKERS`, un-braced.
const MARKERS: &[&str] = &[
    "*/**/Cargo.toml",
    "apps/**/Cargo.toml",
    "apps/**/vite.config.ts",
    "apps/**/astro.config.mjs",
    "apps/**/butler.toml",
    "apps/**/package.json",
    "libs/**/package.json",
];

fn markers() -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for m in MARKERS {
        b.add(Glob::new(m)?);
    }
    Ok(b.build()?)
}

fn dir_of(file: &str) -> &str {
    file.rsplit_once('/').map_or(".", |(d, _)| d)
}

pub struct NativePlugin;

impl Layer for NativePlugin {
    fn name(&self) -> &str {
        "native (tools/nx/plugin.ts port)"
    }

    fn nodes(&self, ctx: &Ctx) -> Result<Vec<Node>> {
        let set = markers()?;
        let files: Vec<&String> = ctx
            .files
            .iter()
            .filter(|f| set.is_match(f.as_str()) && ctx.workspace_root.join(f).is_file())
            .collect();

        let excluded_crates = crates::cargo_exclusions(ctx)?;
        let mut out: Vec<Node> = Vec::new();
        let mut container_apps: BTreeSet<String> = BTreeSet::new();
        let mut workload_apps: BTreeSet<String> = BTreeSet::new();
        let mut polyglot: BTreeSet<String> = BTreeSet::new();
        // `scope:` goes on the FIRST contribution for a dir only — nx concats
        // tags from every contribution.
        let mut tagged: BTreeSet<String> = BTreeSet::new();
        let mut scope_for = |dir: &str| -> Vec<String> {
            if tagged.insert(dir.to_string()) {
                vec![scope::scope_tag(dir)]
            } else {
                vec![]
            }
        };
        let mut crates_by_dir: BTreeMap<String, Option<crates::CargoCrate>> = BTreeMap::new();
        let mut crate_at = |dir: &str| -> Result<Option<crates::CargoCrate>> {
            if !crates_by_dir.contains_key(dir) {
                crates_by_dir.insert(dir.to_string(), crates::read_cargo_crate(ctx, dir)?);
            }
            Ok(crates_by_dir[dir].clone())
        };
        let node = |file: &str, dir: &str, c: Contribution| Node {
            file: file.to_string(),
            projects: BTreeMap::from([(dir.to_string(), c)]),
        };

        for file in files {
            let dir = dir_of(file);

            if file.ends_with("/Cargo.toml")
                && let Some(krate) = crate_at(dir)?
            {
                let excluded = excluded_crates.contains(dir);
                let mut tags = scope_for(dir);
                if !excluded && !crates::has_package_script_gates(ctx, dir)? {
                    tags.push(crates::RUST_TAG.to_string());
                }
                let targets = if excluded {
                    BTreeMap::new()
                } else {
                    let mut t = crates::rust_targets(ctx, &krate, dir)?;
                    t.extend(crates::openapi_targets(ctx, dir, &krate)?);
                    t
                };
                out.push(node(
                    file,
                    dir,
                    Contribution {
                        name: Some(krate.name.clone()),
                        tags,
                        targets,
                        ..Default::default()
                    },
                ));
            }

            if apps::APP_MARKER_FILES
                .iter()
                .any(|m| file.ends_with(&format!("/{m}")))
                && dir.starts_with("apps/")
                && !container_apps.contains(dir)
                && let Some(targets) = apps::container_targets(ctx, dir, &mut crate_at)?
            {
                container_apps.insert(dir.to_string());
                out.push(node(
                    file,
                    dir,
                    Contribution {
                        tags: scope_for(dir),
                        targets,
                        ..Default::default()
                    },
                ));
            }

            if file.ends_with("/butler.toml")
                && dir.starts_with("apps/")
                && !workload_apps.contains(dir)
                && apps::is_workload_app(ctx, dir)?
            {
                workload_apps.insert(dir.to_string());
                let mut targets = apps::tilt_targets(dir);
                targets.extend(apps::k8s_targets(ctx, dir)?);
                out.push(node(
                    file,
                    dir,
                    Contribution {
                        tags: scope_for(dir),
                        targets,
                        ..Default::default()
                    },
                ));
            }

            if !polyglot.contains(dir)
                && (file.ends_with("/Cargo.toml") || file.ends_with("/package.json"))
            {
                let krate = crate_at(dir)?;
                if let Some(targets) = crates::polyglot_targets(
                    ctx,
                    dir,
                    krate.as_ref(),
                    excluded_crates.contains(dir),
                )? {
                    polyglot.insert(dir.to_string());
                    out.push(node(
                        file,
                        dir,
                        Contribution {
                            targets,
                            ..Default::default()
                        },
                    ));
                }
            }
        }

        let missing: Vec<String> = apps::container_extra(ctx)
            .into_iter()
            .filter(|d| !container_apps.contains(d))
            .collect();
        if !missing.is_empty() {
            bail!(
                "butler.toml [container] extra lists {}, which is not an app with a \
                 recognizable kind (a Cargo.toml or a vite.config.ts) under apps/",
                missing.join(", ")
            );
        }
        Ok(out)
    }

    fn dependencies(&self, ctx: &Ctx, projects: &ProjectRoots) -> Result<Vec<Dependency>> {
        crates::crate_dependencies(ctx, projects)
    }
}
