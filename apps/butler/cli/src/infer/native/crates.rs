//! Cargo crates in the project graph: the port of `tools/nx/rust-targets.ts`
//! (`build`, `test`, `doc`, `doc-test`, `run`, `install`), `openapi-targets.ts`
//! (`openapi-gate`), `polyglot-targets.ts` (`fmt`/`lint`, shared with TS
//! packages), the manifest readers `readCargoCrate`/`readCargoExclusions` in
//! `butler-config.ts`, and `createDependencies` in `plugin.ts`. Same rules,
//! same strings — `butler graph verify` holds the two implementations together.
//!
//! Manifests are read directly, never through `cargo metadata`, for the reason
//! the TS gives: the graph has to build where no Rust toolchain is installed
//! (the bun-only CI jobs), and a subprocess would put cargo in front of every
//! graph computation. The parser is a real TOML one rather than the TS's
//! hand-rolled subset, which also makes it read dotted keys
//! (`core_config.workspace = true`), `[dependencies.<name>]` tables and
//! `[target.<cfg>.dependencies]` as the dependencies they are. The TS subset
//! misses those, but `@monodon/rust` contributes the same edges from `cargo
//! metadata` wherever cargo is installed, so they are in every nx graph a
//! developer dumps — omitting them here would be drift.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use eyre::{Result, WrapErr, eyre};
use serde_json::json;
use toml::{Table, Value};

use crate::config::{Json, TargetConfig};
use crate::infer::{Ctx, DepKind, Dependency, ProjectRoots};

/// Tag on every crate node, so a gate can address the Rust half of the graph
/// (`nx affected -t lint test -p tag:lang:rust`). Without that filter the
/// command would also fire the web `lint` scripts, which are mutating
/// `biome check --write` and therefore not gates.
pub const RUST_TAG: &str = "lang:rust";

/// What a crate's manifest says about it.
#[derive(Debug, Clone)]
pub struct CargoCrate {
    /// `[package] name`.
    pub name: String,
    /// A `[[bin]]`, `src/main.rs` or `src/bin/`.
    pub has_binary: bool,
    /// A `[lib]` table or `src/lib.rs` — what `cargo test --doc` needs: rustdoc
    /// collects doctests from the library only, and cargo errors ("no library
    /// targets found") on a bin-only package.
    pub has_library: bool,
    /// `[package] publish = true` (or a non-empty registry list): this binary
    /// is meant to leave the repo. An ABSENT key reads as false, where cargo
    /// reads it as true, deliberately: the flag gates `install`, which writes
    /// into `~/.cargo/bin` outside the workspace, so it is opt-in.
    pub publish: bool,
    /// Dependency keys by table (`[target.*]` variants included).
    pub dependencies: BTreeSet<String>,
    pub dev_dependencies: BTreeSet<String>,
    pub build_dependencies: BTreeSet<String>,
}

impl CargoCrate {
    /// Every dependency name the manifest declares, all three tables merged —
    /// the cheap "can this crate possibly do X" test (`utoipa` -> it may
    /// export an OpenAPI document) and the crate-to-crate edges.
    pub fn dependency_names(&self) -> BTreeSet<&str> {
        self.dependencies
            .iter()
            .chain(&self.dev_dependencies)
            .chain(&self.build_dependencies)
            .map(String::as_str)
            .collect()
    }

    fn declares(&self, name: &str) -> bool {
        self.dependencies.contains(name)
            || self.dev_dependencies.contains(name)
            || self.build_dependencies.contains(name)
    }

    /// Declared under `[dev-dependencies]` and nowhere else: still a graph
    /// edge (it is what makes a test recompile), but it never feeds the
    /// shipped artifact, so image build contexts leave it out.
    fn dev_only(&self, name: &str) -> bool {
        self.dev_dependencies.contains(name)
            && !self.dependencies.contains(name)
            && !self.build_dependencies.contains(name)
    }
}

fn parse_manifest(path: &Path, label: &str) -> Result<Table> {
    let text = std::fs::read_to_string(path).wrap_err_with(|| format!("reading {label}"))?;
    text.parse::<Table>()
        .wrap_err_with(|| format!("parsing {label}"))
}

/// The keys of one dependency table, if `section` has it.
fn dependency_keys(section: &Table, table: &str, out: &mut BTreeSet<String>) {
    if let Some(deps) = section.get(table).and_then(Value::as_table) {
        out.extend(deps.keys().cloned());
    }
}

/// `readCargoCrate`: `None` for a directory with no manifest, and for a
/// virtual manifest (a workspace root with no `[package]`).
pub fn read_cargo_crate(ctx: &Ctx, dir: &str) -> Result<Option<CargoCrate>> {
    let root = ctx.workspace_root.join(dir);
    let path = root.join("Cargo.toml");
    if !path.exists() {
        return Ok(None);
    }
    let label = format!("{dir}/Cargo.toml");
    let manifest = parse_manifest(&path, &label)?;
    let Some(pkg) = manifest.get("package").and_then(Value::as_table) else {
        return Ok(None);
    };
    let Some(name) = pkg.get("name") else {
        return Ok(None);
    };
    let name = name
        .as_str()
        .ok_or_else(|| eyre!("{label}: `[package] name` must be a string"))?;

    let mut sections = vec![&manifest];
    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        sections.extend(targets.values().filter_map(Value::as_table));
    }
    let (mut dependencies, mut dev_dependencies, mut build_dependencies) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for section in sections {
        dependency_keys(section, "dependencies", &mut dependencies);
        dependency_keys(section, "dev-dependencies", &mut dev_dependencies);
        dependency_keys(section, "build-dependencies", &mut build_dependencies);
    }

    Ok(Some(CargoCrate {
        name: name.to_string(),
        has_binary: manifest
            .get("bin")
            .and_then(Value::as_array)
            .is_some_and(|bins| !bins.is_empty())
            || root.join("src/main.rs").exists()
            || root.join("src/bin").exists(),
        // `[lib] path` for a crate that moves it, `src/lib.rs` for the convention.
        has_library: manifest.get("lib").is_some_and(Value::is_table)
            || root.join("src/lib.rs").exists(),
        publish: match pkg.get("publish") {
            Some(Value::Boolean(publish)) => *publish,
            Some(Value::Array(registries)) => !registries.is_empty(),
            _ => false,
        },
        dependencies,
        dev_dependencies,
        build_dependencies,
    }))
}

/// `readCargoExclusions`: the directories the root `Cargo.toml` lists in
/// `[workspace] exclude`. Such a directory has a `[package]`, so it looks like
/// a crate to the manifest glob, but it is not a workspace MEMBER:
/// `cargo <verb> --package <name>` from the root cannot see it, so it gets no
/// cargo targets and — worse than a failing target — no `lang:rust` tag, which
/// would hand it to `task check-rust-affected`. Read once per graph build.
pub fn cargo_exclusions(ctx: &Ctx) -> Result<BTreeSet<String>> {
    let path = ctx.workspace_root.join("Cargo.toml");
    // A workspace without cargo excludes nothing.
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let manifest = parse_manifest(&path, "Cargo.toml")?;
    let Some(raw) = manifest.get("workspace").and_then(|w| w.get("exclude")) else {
        return Ok(BTreeSet::new());
    };
    raw.as_array()
        .ok_or_else(|| eyre!("Cargo.toml: `[workspace] exclude` must be an array"))?
        .iter()
        .map(|dir| {
            dir.as_str()
                .map(str::to_string)
                .ok_or_else(|| eyre!("Cargo.toml: `[workspace] exclude` must list strings"))
        })
        .collect()
}

/// Does a package.json script already own this crate's `lint`/`test`? nx's
/// package.json inference wins over a plugin target, so for the N-API addons —
/// whose deliverable is a JS package and whose `test` is vitest — the cargo
/// gate never runs. Tagging them `lang:rust` would hand a vitest run to
/// `task check-rust-affected`, whose contract is clippy + nextest.
pub fn has_package_script_gates(ctx: &Ctx, dir: &str) -> Result<bool> {
    let path = ctx.workspace_root.join(dir).join("package.json");
    if !path.exists() {
        return Ok(false);
    }
    let label = format!("{dir}/package.json");
    let raw = std::fs::read_to_string(&path).wrap_err_with(|| format!("reading {label}"))?;
    let pkg: Json = serde_json::from_str(&raw).wrap_err_with(|| format!("parsing {label}"))?;
    Ok(pkg
        .get("scripts")
        .and_then(Json::as_object)
        .is_some_and(|scripts| scripts.contains_key("lint") || scripts.contains_key("test")))
}

/// A target literal in nx's JSON shape — kept as JSON so each rule reads like
/// the TS object it ports.
fn target(value: Json) -> Result<TargetConfig> {
    Ok(serde_json::from_value(value)?)
}

/// A cargo gate: cached with NO outputs. cargo and rustdoc write into the
/// shared `dist/target`, which nx cannot fingerprint per crate, so a cache
/// entry means "these inputs passed", never "an artifact was restored".
/// Invalidated by the crate's own files, those of the crates it depends on
/// (`^default`) and the workspace-level compile knobs (`rustGlobals`).
fn cargo_gate(command: String, description: String) -> Result<TargetConfig> {
    target(json!({
        "executor": "nx:run-commands",
        "cache": true,
        "inputs": ["default", "^default", "rustGlobals"],
        "outputs": [],
        "options": {"command": command, "cwd": "{workspaceRoot}"},
        "metadata": {"description": description, "technologies": ["rust"]},
    }))
}

/// `rustTargets`: `build`, `test` and `doc` for every crate, `doc-test` for one
/// with a library, `run` for one with a binary, `install` for a publishable
/// binary. This is the boilerplate every crate used to hand-copy into its
/// `project.json`, derived from the one file that cannot lie about the name.
///
/// These are the per-crate scope `task check-rust-affected` needs (what a diff
/// touched, cached per crate); the workspace-wide `task lint-rust` /
/// `task test-rust` stay single cargo processes and remain the real gate.
pub fn rust_targets(
    _ctx: &Ctx,
    krate: &CargoCrate,
    dir: &str,
) -> Result<BTreeMap<String, TargetConfig>> {
    let name = &krate.name;
    // A library has no artifact worth linking, so `check` is the cheap answer
    // to "does this still compile"; a binary crate is built for real.
    let verb = if krate.has_binary { "build" } else { "check" };
    // `inputs`/`outputs` deliberately absent: the `build` targetDefault wins
    // over an inferred target, so anything set here would be dropped silently.
    let mut build = json!({
        "executor": "nx:run-commands",
        "cache": true,
        "options": {
            "command": format!("cargo {verb} --package {name}"),
            "cwd": "{workspaceRoot}",
        },
        "metadata": {
            "description": format!("cargo {verb} {name}"),
            "technologies": ["rust"],
        },
    });
    if krate.has_binary {
        build["configurations"] = json!({
            "production": {"command": format!("cargo build --package {name} --release")},
        });
    }

    let mut targets = BTreeMap::from([
        ("build".to_string(), target(build)?),
        // `--no-tests=pass`: nextest exits 4 on a crate with no test binaries,
        // a normal state for a crate in isolation (and the permanent one of
        // the cdylib N-API addons) even though `--workspace` always finds some.
        (
            "test".to_string(),
            cargo_gate(
                format!("cargo nextest run --package {name} --no-tests=pass"),
                format!("cargo nextest run {name}"),
            )?,
        ),
        // rustdoc is a third compiler front-end: it resolves every intra-doc
        // link, which neither clippy nor nextest does. `-D warnings` makes that
        // a gate; `--no-deps` because dependency docs are not this crate's to
        // gate. The env is inline so the command string is the whole truth.
        (
            "doc".to_string(),
            cargo_gate(
                format!("RUSTDOCFLAGS=\"-D warnings\" cargo doc --no-deps --package {name}"),
                format!("cargo doc {name} (warnings are errors)"),
            )?,
        ),
    ]);

    // nextest cannot run doctests, so `test` leaves every example uncompiled.
    // Only a `[lib]` gets this: `cargo test --doc` fails on a bin-only package.
    if krate.has_library {
        targets.insert(
            "doc-test".into(),
            cargo_gate(
                format!("cargo test --doc --package {name}"),
                format!("cargo test --doc {name}"),
            )?,
        );
    }

    if !krate.has_binary {
        return Ok(targets);
    }

    // `cargo install` for a binary meant to leave the repo. NOT cached: the
    // artifact lands in `~/.cargo/bin`, outside anything nx fingerprints, so a
    // hit would report success over an old or deleted binary. `--force`
    // because cargo refuses to reinstall the same version, the normal state
    // here; `--locked` builds from the committed Cargo.lock.
    if krate.publish {
        targets.insert(
            "install".into(),
            target(json!({
                "executor": "nx:run-commands",
                "cache": false,
                "options": {
                    "command": format!("cargo install --path {dir} --locked --force"),
                    "cwd": "{workspaceRoot}",
                },
                "metadata": {
                    "description": format!("cargo install {name} into ~/.cargo/bin"),
                    "technologies": ["rust"],
                },
            }))?,
        );
    }

    targets.insert(
        "run".into(),
        target(json!({
            "executor": "nx:run-commands",
            "options": {
                "command": format!("cargo run --package {name}"),
                "cwd": "{workspaceRoot}",
            },
            "configurations": {
                "production": {"command": format!("cargo run --package {name} --release")},
            },
            "metadata": {
                "description": format!("cargo run {name}"),
                "technologies": ["rust"],
            },
        }))?,
    );
    Ok(targets)
}

/// JS `\w`: ASCII word characters.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `/\bfn export_openapi\w*\s*\(/` — the repo's naming convention for the test
/// that writes a document.
fn defines_export_test(text: &str) -> bool {
    const HEAD: &str = "fn export_openapi";
    text.match_indices(HEAD).any(|(at, _)| {
        let boundary = text[..at].chars().next_back().is_none_or(|c| !is_word(c));
        let rest = text[at + HEAD.len()..]
            .trim_start_matches(is_word)
            .trim_start();
        boundary && rest.starts_with('(')
    })
}

/// `/docs\/openapi\/[A-Za-z0-9._-]+\.json/g` — a committed document path as
/// the export call spells it. The class contains `.json`'s characters, so the
/// greedy run backtracks to the LAST `.json` it can end on, as the regex does.
fn document_paths(text: &str, out: &mut BTreeSet<String>) {
    const PREFIX: &str = "docs/openapi/";
    const SUFFIX: &str = ".json";
    let mut from = 0;
    while let Some(found) = text[from..].find(PREFIX) {
        let start = from + found;
        let body = start + PREFIX.len();
        let run = text[body..]
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            .count();
        match (1..=run)
            .rev()
            .find(|&k| text[body + k..].starts_with(SUFFIX))
        {
            Some(k) => {
                let end = body + k + SUFFIX.len();
                out.insert(text[start..end].to_string());
                from = end;
            }
            None => from = start + 1,
        }
    }
}

/// Documents the crate's own sources write, sorted and de-duplicated. Only
/// files that define an export test are searched, so a crate that merely READS
/// the documents (`apps/x/cli` `include_str!`s them) contributes nothing.
fn exported_documents(ctx: &Ctx, dir: &str) -> Result<Vec<String>> {
    let src = ctx.workspace_root.join(dir).join("src");
    if !src.exists() {
        return Ok(Vec::new());
    }
    let mut found = BTreeSet::new();
    let mut pending = vec![src];
    while let Some(at) = pending.pop() {
        let entries =
            std::fs::read_dir(&at).wrap_err_with(|| format!("listing {}", at.display()))?;
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !entry.file_name().to_string_lossy().ends_with(".rs") {
                continue;
            }
            let path = entry.path();
            let bytes =
                std::fs::read(&path).wrap_err_with(|| format!("reading {}", path.display()))?;
            let text = String::from_utf8_lossy(&bytes);
            if defines_export_test(&text) {
                document_paths(&text, &mut found);
            }
        }
    }
    Ok(found.into_iter().collect())
}

/// `openapiTargets`: `openapi-gate` for a crate that WRITES a committed OpenAPI
/// document from a test (`fn export_openapi_*`). The export runs inside the
/// normal suite, so a changed annotation rewrites the document and the suite
/// still passes; without a per-crate gate the CI cargo job (affected crates
/// only) merges a stale document — and `x` derives its whole command tree from
/// those documents. Derived from two facts the code states: the crate depends
/// on `utoipa`, and its sources both define an export test and name a
/// `docs/openapi/*.json` path.
pub fn openapi_targets(
    ctx: &Ctx,
    dir: &str,
    krate: &CargoCrate,
) -> Result<BTreeMap<String, TargetConfig>> {
    // No utoipa, no document: skips the source scan for most crates.
    if !krate.declares("utoipa") {
        return Ok(BTreeMap::new());
    }
    let documents = exported_documents(ctx, dir)?;
    if documents.is_empty() {
        return Ok(BTreeMap::new());
    }
    let name = &krate.name;
    // The committed documents are inputs, not outputs: this target asserts
    // they already are what the annotations produce, so a hand-edit is a
    // cache miss — which is the point.
    let mut inputs = vec![json!("default"), json!("^default"), json!("rustGlobals")];
    inputs.extend(
        documents
            .iter()
            .map(|doc| json!(format!("{{workspaceRoot}}/{doc}"))),
    );
    let gate = target(json!({
        "executor": "nx:run-commands",
        "cache": true,
        "inputs": inputs,
        "outputs": [],
        "options": {
            "commands": [
                format!("cargo test --package {name} export_openapi"),
                format!("git diff --exit-code -- {}", documents.join(" ")),
            ],
            // The diff reads what the test just wrote.
            "parallel": false,
            "cwd": "{workspaceRoot}",
        },
        "metadata": {
            "description": format!(
                "Fail if {} has drifted from {name}'s annotations",
                documents.join(", ")
            ),
            "technologies": ["rust"],
        },
    }))?;
    Ok(BTreeMap::from([("openapi-gate".to_string(), gate)]))
}

/// `polyglotTargets`: `fmt` and `lint` — the names both ecosystems answer to —
/// COMPOSED from what the directory holds (a crate, a TS package, or both, as
/// for the ts-rs bindings and N-API addons), so one name means both
/// toolchains. `None` when the directory is neither. biome is the only JS/TS
/// linter/formatter and has one root config, so a per-project run is the same
/// tool with a narrower path.
///
/// `excluded` marks a `[workspace] exclude`d crate: cargo cannot resolve its
/// package from the root, so it is formatted by manifest path (for such a
/// crate this is the ONLY formatter that reaches it) and never clippy'd (that
/// would build a wasm32-only crate for the host).
pub fn polyglot_targets(
    ctx: &Ctx,
    dir: &str,
    krate: Option<&CargoCrate>,
    excluded: bool,
) -> Result<Option<BTreeMap<String, TargetConfig>>> {
    let is_package = ctx.workspace_root.join(dir).join("package.json").exists();
    if krate.is_none() && !is_package {
        return Ok(None);
    }

    let mut fmt: Vec<String> = Vec::new();
    let mut lint: Vec<String> = Vec::new();
    let mut technologies: Vec<&str> = Vec::new();
    let mut lint_inputs = vec![json!("default"), json!("^default")];

    if let Some(krate) = krate {
        // `cargo sort` rides along because `task fmt-check-rust` gates rustfmt
        // AND dependency-table order; same flags as the task, so neither
        // rewrites what the other wrote.
        fmt.push(if excluded {
            format!("cargo fmt --manifest-path {dir}/Cargo.toml --all")
        } else {
            format!("cargo fmt --package {}", krate.name)
        });
        fmt.push(format!("cargo sort {dir}"));
        technologies.push("rust");
        if !excluded {
            // Same flags as `task lint-rust`, one crate at a time.
            lint.push(format!(
                "cargo clippy --package {} --all-targets -- -D warnings",
                krate.name
            ));
            lint_inputs.push(json!("rustGlobals"));
        }
    }

    if is_package {
        // `--linter-enabled=false`: `biome check --write` exits 1 on an
        // unfixable lint diagnostic, and a formatter failing on lint is a gate
        // wearing the wrong name. `biome ci` is the (read-only) gate.
        fmt.push(format!(
            "bunx biome check --write --linter-enabled=false {dir}"
        ));
        lint.push(format!("bunx biome ci {dir}"));
        technologies.push("typescript");
        lint_inputs.push(json!("{workspaceRoot}/biome.json"));
        lint_inputs.push(json!({"externalDependencies": ["@biomejs/biome"]}));
    }

    let stack = technologies.join(" + ");
    let mut targets = BTreeMap::from([(
        "fmt".to_string(),
        // NEVER cached: rewriting files in place IS the output, and a hit would
        // report success over a tree someone has since reverted.
        target(json!({
            "executor": "nx:run-commands",
            "cache": false,
            "options": {
                "commands": fmt,
                "cwd": "{workspaceRoot}",
                // `cargo sort` reads the manifest `cargo fmt` may have rewritten.
                "parallel": false,
            },
            "metadata": {
                "description": format!("format {dir} ({stack})"),
                "technologies": technologies,
            },
        }))?,
    )]);

    if !lint.is_empty() {
        // Cached with no outputs, like the cargo gates: an entry means "these
        // inputs passed clippy/biome". `nx.json`'s `lint` targetDefault sets
        // only `cache`, so these inputs survive.
        targets.insert(
            "lint".into(),
            target(json!({
                "executor": "nx:run-commands",
                "cache": true,
                "inputs": lint_inputs,
                "outputs": [],
                "options": {"commands": lint, "cwd": "{workspaceRoot}", "parallel": true},
                "metadata": {
                    "description": format!("lint {dir} ({stack})"),
                    "technologies": technologies,
                },
            }))?,
        );
    }
    Ok(Some(targets))
}

/// `createDependencies`: crate-to-crate edges, read from the manifests. A
/// workspace dependency is declared BY NAME (`core_config = { workspace =
/// true }`), so matching dependency keys against the known crate names is the
/// whole resolution — no path arithmetic, no subprocess. `@monodon/rust` adds
/// the same edges only where `cargo metadata` runs; without these, a bun-only
/// CI job would lose every crate edge and `nx affected` would miss the app
/// whose library a diff touched.
pub fn crate_dependencies(ctx: &Ctx, projects: &ProjectRoots) -> Result<Vec<Dependency>> {
    // Crate name -> project name, for the projects the graph already has.
    let mut project_by_crate: BTreeMap<String, &str> = BTreeMap::new();
    let mut crates: Vec<(&str, &str, CargoCrate)> = Vec::new();
    for (project, root) in projects {
        let Some(krate) = read_cargo_crate(ctx, root)? else {
            continue;
        };
        project_by_crate.insert(krate.name.clone(), project);
        crates.push((project, root, krate));
    }

    let mut out = Vec::new();
    for (project, root, krate) in &crates {
        for dependency in krate.dependency_names() {
            let Some(&target) = project_by_crate.get(dependency) else {
                continue;
            };
            if target == *project {
                continue;
            }
            out.push(Dependency {
                source: (*project).to_string(),
                target: target.to_string(),
                kind: DepKind::Static,
                source_file: Some(format!("{root}/Cargo.toml")),
                dev: krate.dev_only(dependency),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::{self, native::NativePlugin};

    #[test]
    fn export_test_needs_the_whole_convention() {
        assert!(defines_export_test(
            "#[test]\nfn export_openapi_todo_v1 () {"
        ));
        assert!(defines_export_test("fn export_openapi(){"));
        // `\b`: part of a longer identifier is not the convention.
        assert!(!defines_export_test("fn myfn export_openapi() {"));
        assert!(!defines_export_test("fn export_openapi_v1<T>() {"));
        assert!(!defines_export_test(
            "let fn_export_openapi = 1; // fn export_openapi"
        ));
    }

    #[test]
    fn document_paths_backtrack_like_the_regex() {
        let mut found = BTreeSet::new();
        document_paths(
            "write(\"../../docs/openapi/todo.v1.json.bak\"); \
             docs/openapi/docs/openapi/zerg.json docs/openapi/.json docs/openapi/x.jsonl",
            &mut found,
        );
        let found: Vec<&str> = found.iter().map(String::as_str).collect();
        assert_eq!(
            found,
            [
                "docs/openapi/todo.v1.json",
                "docs/openapi/x.json",
                "docs/openapi/zerg.json",
            ]
        );
    }

    /// Throwaway workspace; removed on drop.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "butler-crates-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            for (rel, content) in files {
                let path = dir.join(rel);
                std::fs::create_dir_all(path.parent().expect("file has a parent"))
                    .expect("scratch parents");
                std::fs::write(&path, content).expect("scratch file");
            }
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn excluded_crates_and_dev_only_edges() {
        let files = [
            (
                "Cargo.toml",
                "[workspace]\nmembers = ['libs/*']\nexclude = ['libs/wasm']\n",
            ),
            ("libs/core/Cargo.toml", "[package]\nname = 'core_lib'\n"),
            ("libs/core/src/lib.rs", ""),
            (
                "libs/testing/Cargo.toml",
                "[package]\nname = 'test_utils'\n",
            ),
            ("libs/testing/src/lib.rs", ""),
            (
                "libs/app/Cargo.toml",
                "[package]\nname = 'app_bin'\n\n[dependencies]\ncore_lib.workspace = true\n\n\
                 [dev-dependencies]\ncore_lib = { workspace = true }\ntest_utils = { workspace = true }\n",
            ),
            ("libs/app/src/main.rs", ""),
            (
                "libs/wasm/Cargo.toml",
                "[package]\nname = 'wasm_app'\n\n[dependencies.core_lib]\npath = '../core'\n",
            ),
            ("libs/wasm/src/main.rs", ""),
        ];
        let scratch = Scratch::new(&files);
        let listed: Vec<String> = files.iter().map(|(rel, _)| (*rel).to_string()).collect();
        let ctx = Ctx {
            workspace_root: &scratch.0,
            files: &listed,
            settings: None,
            overrides: None,
        };
        let graph = infer::build(&ctx, &[&NativePlugin], &BTreeMap::new()).expect("graph builds");

        // Not a workspace member: formatted by manifest path, nothing else, and
        // no `lang:rust` tag to hand it to the cargo gates.
        let wasm = &graph.projects["wasm_app"];
        assert!(!wasm.tags.iter().any(|t| t == RUST_TAG));
        assert_eq!(wasm.targets.keys().collect::<Vec<_>>(), ["fmt"]);
        assert_eq!(
            wasm.targets["fmt"].options.as_ref().expect("options")["commands"][0],
            "cargo fmt --manifest-path libs/wasm/Cargo.toml --all"
        );
        // It still depends on what it declares, `[dependencies.<name>]` included.
        assert!(wasm.build_deps.contains("core_lib"));

        let app = &graph.projects["app_bin"];
        assert!(app.tags.iter().any(|t| t == RUST_TAG));
        // A dev-dependency is an edge, but only a normal one feeds the build —
        // and a crate declared in both tables is a normal dependency.
        assert_eq!(
            app.deps.iter().map(String::as_str).collect::<Vec<_>>(),
            ["core_lib", "test_utils"]
        );
        assert_eq!(
            app.build_deps
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["core_lib"]
        );
    }
}
