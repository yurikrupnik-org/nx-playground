//! Port of the `@nxlv/python` 23.1.1 executors this repo uses: `add`,
//! `build`, `install`, `lock`, `remove`, `ruff-check`, `ruff-format`,
//! `run-commands`, `sync`, `update` (package `dist/executors/*` plus the uv
//! provider in `dist/provider/uv`).
//!
//! Every executor starts with `getProvider(context.root, …, context)`. With no
//! plugin options (executors never pass any), the provider is uv when the
//! project's `pyproject.toml` has a `[project]` table and no `[tool.poetry]`
//! (or, without a project `pyproject.toml`, when the root has `uv.lock`), and
//! poetry otherwise. Only uv is ported: a poetry project is a hard error. The
//! provider is in *workspace* mode when the workspace root has `uv.lock`: uv
//! then runs at the root with `--project <root>`, otherwise inside the
//! project, followed by `uv sync` in every project whose
//! `[tool.uv.sources]` names it (`syncDependents`).
//!
//! Plain `uv` invocations become [`Step::Exec`] (`runUv` spawns without a
//! shell) and the ruff executors a [`Step::Shell`] (`provider.run` spawns with
//! `shell: true`, arguments joined by spaces, unquoted). The uv provider's
//! `checkPrerequisites` (`command-exists uv`) is a [`Step::Native`] in front
//! of them, as is the lazily created virtualenv of
//! `installDependenciesIfNotExists`. `build` is one native step: it works in a
//! random temp folder and reads `uv export` output before rewriting the
//! manifest, none of which is known when planning.
//!
//! Manifests are read the way `@iarna/toml` 2.2.5 reads them (TOML 0.5:
//! mixed-type arrays are rejected; keys keep document order, integer-like keys
//! first as in any JS object) and written with a line-for-line port of its
//! `stringify`, so the build folder's `pyproject.toml` is byte-identical to
//! the one nx writes. Known gaps, all loud: TOML datetimes are refused (their
//! JS `Date` rendering is not reproduced), and documents using TOML 1.x-only
//! syntax that `@iarna/toml` rejects are accepted.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;

use eyre::{Result, bail, eyre};
use globset::{GlobBuilder, GlobMatcher};
use serde_json::json;

use super::args::js_string;
use super::schema::combine_options;
use super::{NativeCtx, NativeFn, Plan, PlanCtx, Step, StepOutput, shell_quote};
use crate::config::{Json, JsonMap};

type Env = BTreeMap<String, String>;

/// Plan `@nxlv/python:<executor>` for one task.
pub fn plan(executor: &str, ctx: &PlanCtx<'_>) -> Result<Plan> {
    let task = format!("{}:{}", ctx.project.name, ctx.target);
    match executor {
        "add" => add(ctx, &task),
        "build" => build(ctx, &task),
        // `provider.sync` is `provider.install`; the schemas only differ in
        // descriptions.
        "install" | "sync" => install(ctx, &task),
        "lock" => lock(ctx, &task),
        "remove" => remove(ctx, &task),
        "ruff-check" => ruff(
            ctx,
            &task,
            &ruff_check_schema(),
            "check",
            "lintFilePatterns",
            &[("--fix", "fix"), ("--exit-zero", "exitZero")],
        ),
        "ruff-format" => ruff(
            ctx,
            &task,
            &ruff_format_schema(),
            "format",
            "filePatterns",
            &[("--check", "check")],
        ),
        "run-commands" => run_commands(ctx, &task),
        "update" => update(ctx, &task),
        "flake8"
        | "package-dependencies"
        | "package-project"
        | "publish"
        | "sls-deploy"
        | "sls-package"
        | "tox" => bail!(
            "{task}: @nxlv/python:{executor} is not ported to butler; run the target through nx"
        ),
        other => bail!("{task}: `@nxlv/python:{other}` is not an executor of @nxlv/python 23.1.1"),
    }
}

// ---------------------------------------------------------------------------
// Executor schemas (the properties of each `schema.json`, verbatim).

fn add_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "$default": {"$source": "argv", "index": 0}},
            "args": {"type": "string"},
            "local": {"type": "boolean", "default": false},
            "group": {"type": "string"},
            "extras": {"type": "array", "items": {"type": "string"}},
            "installDependenciesIfNotExists": {"type": "boolean", "default": false}
        },
        "required": ["name"]
    })
}

fn build_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "silent": {"type": "boolean", "default": false},
            "outputPath": {"type": "string"},
            "keepBuildFolder": {"type": "boolean", "default": false},
            "ignorePaths": {
                "type": "array",
                "default": [".venv", ".tox", "tests"],
                "items": {"type": "string"}
            },
            "devDependencies": {"type": "boolean", "default": false},
            "lockedVersions": {"type": "boolean", "default": true},
            "bundleLocalDependencies": {"type": "boolean", "default": true},
            "customSourceName": {"type": "string", "default": "private"},
            "customSourceUrl": {"type": "string"},
            "format": {"type": "string", "enum": ["sdist", "wheel"]},
            "installDependenciesIfNotExists": {"type": "boolean", "default": false}
        },
        "required": ["outputPath"]
    })
}

/// `install` and `sync` (identical properties).
fn install_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "silent": {"type": "boolean", "default": false},
            "args": {"type": "string"},
            "cacheDir": {"type": "string"},
            "verbose": {"type": "boolean", "default": false},
            "debug": {"type": "boolean", "default": false}
        },
        "required": []
    })
}

fn lock_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "silent": {"type": "boolean", "default": false},
            "args": {"type": "string"},
            "cacheDir": {"type": "string"},
            "verbose": {"type": "boolean", "default": false},
            "debug": {"type": "boolean", "default": false},
            "update": {"type": "boolean", "default": false}
        },
        "required": []
    })
}

fn remove_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "args": {"type": "string"},
            "local": {"type": "boolean", "default": false},
            "installDependenciesIfNotExists": {"type": "boolean", "default": false}
        },
        "required": ["name"]
    })
}

fn update_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "args": {"type": "string"},
            "local": {"type": "boolean", "default": false},
            "installDependenciesIfNotExists": {"type": "boolean", "default": false}
        },
        "required": []
    })
}

fn ruff_check_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "lintFilePatterns": {"type": "array", "items": {"type": "string"}},
            "fix": {"type": "boolean", "default": false},
            "exitZero": {"type": "boolean", "default": false},
            "installDependenciesIfNotExists": {"type": "boolean", "default": false},
            "__unparsed__": {
                "hidden": true,
                "type": "array",
                "items": {"type": "string"},
                "$default": {"$source": "unparsed"}
            }
        },
        "required": ["lintFilePatterns"]
    })
}

fn ruff_format_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "filePatterns": {"type": "array", "items": {"type": "string"}},
            "check": {"type": "boolean", "default": false},
            "installDependenciesIfNotExists": {"type": "boolean", "default": false},
            "__unparsed__": {
                "hidden": true,
                "type": "array",
                "items": {"type": "string"},
                "$default": {"$source": "unparsed"}
            }
        },
        "required": ["filePatterns"]
    })
}

/// @nxlv's own run-commands schema: a narrower copy of nx's plus
/// `installDependenciesIfNotExists`. nx combines the options under THIS
/// schema, not nx's.
fn run_commands_schema() -> Json {
    json!({
        "type": "object",
        "properties": {
            "commands": {
                "type": "array",
                "items": {
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "command": {"type": "string"},
                                "forwardAllArgs": {"type": "boolean"},
                                "prefix": {"type": "string"},
                                "color": {
                                    "type": "string",
                                    "enum": ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"]
                                },
                                "bgColor": {
                                    "type": "string",
                                    "enum": ["bgBlack", "bgRed", "bgGreen", "bgYellow", "bgBlue", "bgMagenta", "bgCyan", "bgWhite"]
                                },
                                "description": {"type": "string"}
                            },
                            "additionalProperties": false,
                            "required": ["command"]
                        },
                        {"type": "string"}
                    ]
                }
            },
            "command": {"type": "string"},
            "parallel": {"type": "boolean", "default": true},
            "readyWhen": {"type": "string"},
            "args": {"type": "string"},
            "envFile": {"type": "string"},
            "color": {"type": "boolean", "default": false},
            "cwd": {"type": "string"},
            "installDependenciesIfNotExists": {"type": "boolean", "default": false},
            "__unparsed__": {
                "hidden": true,
                "type": "array",
                "items": {"type": "string"},
                "$default": {"$source": "unparsed"}
            }
        },
        "additionalProperties": true,
        "oneOf": [{"required": ["commands"]}, {"required": ["command"]}]
    })
}

// ---------------------------------------------------------------------------
// Executors.

/// `ruff-check` / `ruff-format`: `extractBooleanFlag` pulls each boolean flag
/// out of the raw args (the raw form wins over the parsed option), the rest of
/// the raw args follow the file patterns, and the enabled flags go last:
/// `uv run ruff format test_cli1 tests --check`.
fn ruff(
    ctx: &PlanCtx<'_>,
    task: &str,
    schema: &Json,
    sub: &str,
    patterns_key: &str,
    flags: &[(&str, &str)],
) -> Result<Plan> {
    let opts = combine_options(ctx, schema)?;
    let mut unparsed = match opts.get("__unparsed__") {
        Some(Json::Array(items)) => items.iter().map(js_string).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let mut enabled = Vec::with_capacity(flags.len());
    for (flag, key) in flags {
        let on =
            extract_boolean_flag(&mut unparsed, flag).unwrap_or_else(|| truthy(opts.get(*key)));
        enabled.push((*flag, on));
    }
    let mut args = vec!["ruff".to_string(), sub.to_string()];
    args.extend(js_concat(opts.get(patterns_key)));
    args.extend(unparsed);
    args.extend(
        enabled
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(flag, _)| flag.to_string()),
    );

    let uv = Uv::resolve(ctx, task)?;
    let venv = uv.activate_venv(
        ctx,
        task,
        truthy(opts.get("installDependenciesIfNotExists")),
    )?;
    // `provider.run`: activate, check for uv, then `uv run …` through a shell
    // in the project root.
    let mut steps: Vec<Step> = venv.install.into_iter().collect();
    steps.push(check_uv_step(venv.delta.clone()));
    let words: Vec<&str> = std::iter::once("run")
        .chain(args.iter().map(String::as_str))
        .collect();
    steps.push(Step::Shell {
        script: format!("uv {}", words.join(" ")),
        cwd: uv.root.clone(),
        env: venv.delta,
    });
    Ok(Plan {
        steps,
        parallel: false,
    })
}

/// `@nxlv/python:run-commands`: nx's run-commands with the virtualenv
/// activated first. The options nx combined under @nxlv's schema go to the
/// run-commands planner as-is (overrides already applied), minus
/// `installDependenciesIfNotExists`, and with the activated environment as
/// the task environment so its `PATH` handling starts from the venv's.
fn run_commands(ctx: &PlanCtx<'_>, task: &str) -> Result<Plan> {
    let mut opts = combine_options(ctx, &run_commands_schema())?;
    let uv = Uv::resolve(ctx, task)?;
    let install = truthy(opts.remove("installDependenciesIfNotExists").as_ref());
    let venv = uv.activate_venv(ctx, task, install)?;
    let overrides = JsonMap::new();
    let sub = PlanCtx {
        workspace_root: ctx.workspace_root,
        graph: ctx.graph,
        project: ctx.project,
        target: ctx.target,
        configuration: ctx.configuration,
        options: &opts,
        overrides: &overrides,
        unparsed: ctx.unparsed,
        env: &venv.full,
    };
    let inner = super::run_commands::run_commands(&sub)?;
    // Its steps carry only their delta over `venv.full`; the venv variables
    // are part of what they run with.
    let steps: Vec<Step> = inner
        .steps
        .into_iter()
        .map(|s| layer_env(s, &venv.delta))
        .collect();
    match venv.install {
        None => Ok(Plan {
            steps,
            parallel: inner.parallel,
        }),
        Some(install) if !inner.parallel || steps.len() <= 1 => Ok(Plan {
            steps: std::iter::once(install).chain(steps).collect(),
            parallel: false,
        }),
        Some(_) => bail!(
            "{task}: installDependenciesIfNotExists creates the virtualenv before the parallel \
             commands start; a butler plan cannot order one step before a parallel group — set \
             `parallel: false` or run the target through nx"
        ),
    }
}

/// `lock`: `uv lock [--upgrade] [args…] [-v|-vvv] [--cache-dir d]` at the
/// root (workspace) or in the project. `args` is split on single spaces with
/// empty pieces kept, as the JS does here (unlike add/install/remove).
fn lock(ctx: &PlanCtx<'_>, task: &str) -> Result<Plan> {
    let opts = combine_options(ctx, &lock_schema())?;
    let uv = Uv::resolve(ctx, task)?;
    let mut args = vec!["lock".to_string()];
    if truthy(opts.get("update")) {
        args.push("--upgrade".into());
    }
    if let Some(extra) = opts.get("args").filter(|a| truthy(Some(*a))) {
        args.extend(js_string(extra).split(' ').map(str::to_string));
    }
    push_verbosity(&mut args, &opts);
    push_cache_dir(&mut args, &opts);
    Ok(Plan {
        steps: vec![
            check_uv_step(Env::new()),
            uv_step(args, uv.cwd(), &Env::new()),
        ],
        parallel: false,
    })
}

/// `install` / `sync`: `uv sync [-v|-vvv] [args…] [--cache-dir d]`.
fn install(ctx: &PlanCtx<'_>, task: &str) -> Result<Plan> {
    let opts = combine_options(ctx, &install_schema())?;
    let uv = Uv::resolve(ctx, task)?;
    let mut args = vec!["sync".to_string()];
    push_verbosity(&mut args, &opts);
    args.extend(split_args(opts.get("args")));
    push_cache_dir(&mut args, &opts);
    Ok(Plan {
        steps: vec![
            check_uv_step(Env::new()),
            uv_step(args, uv.cwd(), &Env::new()),
        ],
        parallel: false,
    })
}

/// `add`: `uv add <name> [--group g] [--extra e…] [args…]`, plus
/// `--project <root>` at the workspace root; outside a workspace a `local`
/// dependency is added as `--editable <relative path>`.
fn add(ctx: &PlanCtx<'_>, task: &str) -> Result<Plan> {
    let opts = combine_options(ctx, &add_schema())?;
    let uv = Uv::resolve(ctx, task)?;
    let name = required_string(&opts, "name", task)?;
    let mut args = vec!["add".to_string()];
    if !uv.is_workspace && truthy(opts.get("local")) {
        let dep = ctx
            .graph
            .projects
            .get(&name)
            .ok_or_else(|| eyre!("{task}: project {name} not found in the Nx workspace"))?;
        let cwd = ws_string(ctx.workspace_root)?;
        args.push("--editable".into());
        args.push(jspath::relative(&cwd, &uv.root, &dep.root));
    } else {
        args.push(name);
    }
    if let Some(group) = opts.get("group").filter(|g| truthy(Some(*g))) {
        args.push("--group".into());
        args.push(js_string(group));
    }
    if let Some(Json::Array(extras)) = opts.get("extras") {
        for extra in extras {
            args.push("--extra".into());
            args.push(js_string(extra));
        }
    }
    args.extend(split_args(opts.get("args")));
    if uv.is_workspace {
        args.push("--project".into());
        args.push(uv.root.clone());
    }
    let mut steps = vec![
        check_uv_step(Env::new()),
        uv_step(args, uv.cwd(), &Env::new()),
    ];
    steps.extend(uv.sync_dependents(ctx, task)?);
    Ok(Plan {
        steps,
        parallel: false,
    })
}

/// `update`: `uv lock --upgrade-package <name> [--project <root>]`, then
/// `uv sync`.
fn update(ctx: &PlanCtx<'_>, task: &str) -> Result<Plan> {
    let opts = combine_options(ctx, &update_schema())?;
    let uv = Uv::resolve(ctx, task)?;
    // `name` is optional in the schema, but the JS hands `undefined` to uv.
    let name = required_string(&opts, "name", task)?;
    let mut args = vec!["lock".to_string(), "--upgrade-package".into(), name];
    if uv.is_workspace {
        args.push("--project".into());
        args.push(uv.root.clone());
    }
    let mut steps = vec![
        check_uv_step(Env::new()),
        uv_step(args, uv.cwd(), &Env::new()),
        uv_step(vec!["sync".into()], uv.cwd(), &Env::new()),
    ];
    steps.extend(uv.sync_dependents(ctx, task)?);
    Ok(Plan {
        steps,
        parallel: false,
    })
}

/// `remove`: `uv remove <name> [--project <root>] [args…]`.
fn remove(ctx: &PlanCtx<'_>, task: &str) -> Result<Plan> {
    let opts = combine_options(ctx, &remove_schema())?;
    let uv = Uv::resolve(ctx, task)?;
    let name = required_string(&opts, "name", task)?;
    let mut args = vec!["remove".to_string(), name];
    if uv.is_workspace {
        args.push("--project".into());
        args.push(uv.root.clone());
    }
    args.extend(split_args(opts.get("args")));
    let mut steps = vec![
        check_uv_step(Env::new()),
        uv_step(args, uv.cwd(), &Env::new()),
    ];
    steps.extend(uv.sync_dependents(ctx, task)?);
    Ok(Plan {
        steps,
        parallel: false,
    })
}

/// `build`: see [`BuildCfg`] and [`run_build`].
fn build(ctx: &PlanCtx<'_>, task: &str) -> Result<Plan> {
    let opts = combine_options(ctx, &build_schema())?;
    let uv = Uv::resolve(ctx, task)?;
    if opts.get("lockedVersions") == Some(&Json::Bool(true))
        && opts.get("bundleLocalDependencies") == Some(&Json::Bool(false))
    {
        bail!(
            "{task}: Not supported operations, you cannot use lockedVersions without \
             bundleLocalDependencies"
        );
    }
    let ignore_raw: Vec<String> = match opts.get("ignorePaths") {
        Some(Json::Array(items)) => items.iter().map(js_string).collect(),
        _ => bail!("{task}: `ignorePaths` must be an array"),
    };
    let ignore = ignore_raw
        .iter()
        .map(|p| IgnorePattern::new(p).map_err(|e| eyre!("{task}: ignorePaths: {e}")))
        .collect::<Result<Vec<_>>>()?;
    let format = match opts
        .get("format")
        .filter(|f| truthy(Some(*f)))
        .map(js_string)
    {
        Some(f) if f == "sdist" || f == "wheel" => Some(f),
        Some(f) => bail!("{task}: `format` must be sdist or wheel, got `{f}`"),
        None => None,
    };
    // Build-target options of every project, for the non-locked resolver's
    // `publish` / `customSourceUrl` lookups of local dependencies.
    let projects = ctx
        .graph
        .projects
        .values()
        .map(|p| {
            let build = p.targets.get("build").and_then(|t| t.options.clone());
            (p.root.clone(), build)
        })
        .collect();
    let cfg = Arc::new(BuildCfg {
        project_name: ctx.project.name.clone(),
        project_root: uv.root.clone(),
        is_workspace: uv.is_workspace,
        ignore,
        ignore_raw,
        build_folder: opts
            .get("buildFolder")
            .filter(|b| !b.is_null())
            .map(js_string),
        locked_versions: truthy(opts.get("lockedVersions")),
        dev_dependencies: truthy(opts.get("devDependencies")),
        bundle_local: opts.get("bundleLocalDependencies") == Some(&Json::Bool(true)),
        output_path: required_string(&opts, "outputPath", task)?,
        format,
        skip_build: truthy(opts.get("skipBuild")),
        keep_build_folder: truthy(opts.get("keepBuildFolder")),
        silent: truthy(opts.get("silent")),
        projects,
    });
    let label = cfg.label();
    let run: NativeFn = Arc::new(move |n: &NativeCtx<'_>| Ok(run_build(&cfg, n)));
    Ok(Plan {
        steps: vec![check_uv_step(Env::new()), Step::Native { label, run }],
        parallel: false,
    })
}

// ---------------------------------------------------------------------------
// The uv provider.

struct Uv {
    /// Project root, workspace-root-relative.
    root: String,
    /// `uv.lock` at the workspace root.
    is_workspace: bool,
}

impl Uv {
    /// `getProvider(context.root, …, context)` (see the module docs).
    fn resolve(ctx: &PlanCtx<'_>, task: &str) -> Result<Uv> {
        let ws = ctx.workspace_root;
        let root = ctx.project.root.clone();
        let pyproject = ws.join(jspath::join(&[&root, "pyproject.toml"]));
        let (uv, poetry) = if pyproject.exists() {
            let data = pyproject_data(&pyproject)?;
            let has_poetry = match data.get("tool") {
                None | Some(Tv::Undefined | Tv::Null) => false,
                Some(Tv::Table(tool)) => tool.get("poetry").is_some(),
                Some(Tv::Arr(_)) => false,
                Some(other) => bail!(
                    "{task}: {}: `tool` is a {} ('poetry' in … throws in the JS)",
                    pyproject.display(),
                    other.kind()
                ),
            };
            (data.get("project").is_some() && !has_poetry, has_poetry)
        } else {
            (ws.join("uv.lock").exists(), ws.join("poetry.lock").exists())
        };
        if uv && poetry {
            bail!("{task}: Both poetry.lock and uv.lock files found. Please remove one of them.");
        }
        if !uv {
            bail!(
                "{task}: @nxlv/python selects its poetry provider for {root} (no uv project); \
                 butler ports only the uv provider — run the target through nx"
            );
        }
        Ok(Uv {
            root,
            is_workspace: ws.join("uv.lock").exists(),
        })
    }

    /// Where uv runs: `context.root` in a workspace, else the project root.
    fn cwd(&self) -> &str {
        if self.is_workspace { "." } else { &self.root }
    }

    /// `BaseProvider.activateVenv`. Nothing happens when `VIRTUAL_ENV` is
    /// already set. In a workspace whose root `pyproject.toml` sets
    /// `[tool.nx] autoActivate`, the root `.venv` is activated; with
    /// `installIfNotExists`, the workspace (or project) `.venv` is created by
    /// `uv sync` when missing — decided when the step runs — and activated.
    /// Activation sets `VIRTUAL_ENV`, prepends `<venv>/bin` to `PATH` and
    /// deletes `PYTHONHOME`.
    fn activate_venv(
        &self,
        ctx: &PlanCtx<'_>,
        task: &str,
        install_if_not_exists: bool,
    ) -> Result<Venv> {
        let ws = ws_string(ctx.workspace_root)?;
        let mut env = ctx.env.clone();
        let mut install = None;
        if ctx.env.get("VIRTUAL_ENV").is_none_or(String::is_empty) {
            let root_pyproject = ctx.workspace_root.join("pyproject.toml");
            if self.is_workspace && root_pyproject.exists() {
                let root = parse_toml_file(&root_pyproject)?;
                let auto = match root.get("tool") {
                    None | Some(Tv::Undefined | Tv::Null) => bail!(
                        "{task}: the root pyproject.toml has no [tool] table; @nxlv/python's \
                         activateVenv reads `tool.nx.autoActivate` unguarded and fails with a \
                         TypeError"
                    ),
                    Some(tool) => tool.prop("nx").and_then(|nx| nx.prop("autoActivate")),
                };
                if auto.is_some_and(Tv::truthy) {
                    set_venv(&mut env, &jspath::resolve(&ws, &[&ws, ".venv"]));
                }
            }
            if install_if_not_exists {
                let (base, shown) = if self.is_workspace {
                    (ws.clone(), ".".to_string())
                } else {
                    (self.root.clone(), self.root.clone())
                };
                let venv = jspath::resolve(&ws, &[&base, ".venv"]);
                install = Some(venv_install_step(
                    base,
                    shown,
                    venv.clone(),
                    diff_env(ctx.env, &env),
                ));
                set_venv(&mut env, &venv);
            }
        }
        if ctx.env.contains_key("PYTHONHOME") && !env.contains_key("PYTHONHOME") {
            bail!(
                "{task}: @nxlv/python activates the virtualenv by deleting PYTHONHOME from the \
                 environment; butler steps can set variables but not unset them — unset \
                 PYTHONHOME or run the target through nx"
            );
        }
        Ok(Venv {
            delta: diff_env(ctx.env, &env),
            full: env,
            install,
        })
    }

    /// `syncDependents` outside a workspace: `uv sync` in every project whose
    /// `[tool.uv.sources]` names this project's package, recursively, each
    /// project once. Decided when planning: the add/remove/update that
    /// precedes it only edits this project's manifest, and this project is
    /// always already "updated", so the result is the same as the JS's
    /// run-time walk. Projects are visited in project-name order (nx walks
    /// its project map in insertion order).
    fn sync_dependents(&self, ctx: &PlanCtx<'_>, task: &str) -> Result<Vec<Step>> {
        let mut steps = Vec::new();
        if !self.is_workspace {
            let mut updated = Vec::new();
            self.sync_dependents_of(ctx, task, &ctx.project.name, &mut updated, &mut steps)?;
        }
        Ok(steps)
    }

    fn sync_dependents_of(
        &self,
        ctx: &PlanCtx<'_>,
        task: &str,
        project: &str,
        updated: &mut Vec<String>,
        steps: &mut Vec<Step>,
    ) -> Result<()> {
        updated.push(project.to_string());
        for dep in get_dependents(ctx, task, project)? {
            if updated.contains(&dep) {
                continue;
            }
            let root = ctx.graph.get(&dep)?.root.clone();
            steps.push(uv_step(vec!["sync".into()], &root, &Env::new()));
            self.sync_dependents_of(ctx, task, &dep, updated, steps)?;
        }
        Ok(())
    }
}

/// `UVProvider.getDependents`, non-workspace branch.
fn get_dependents(ctx: &PlanCtx<'_>, task: &str, project: &str) -> Result<Vec<String>> {
    let ws = ctx.workspace_root;
    let root = &ctx.graph.get(project)?.root;
    let own = pyproject_data(&ws.join(jspath::join(&[root, "pyproject.toml"])))?;
    let name = match own.get("project") {
        Some(Tv::Table(p)) => p
            .get("name")
            .map_or_else(|| "undefined".to_string(), Tv::js_str),
        _ => bail!(
            "{task}: {root}/pyproject.toml has no [project] table (syncDependents reads \
             `project.name` unguarded)"
        ),
    };
    let mut out = Vec::new();
    for (pname, p) in &ctx.graph.projects {
        let path = ws.join(jspath::join(&[&p.root, "pyproject.toml"]));
        if path.exists() {
            let data = pyproject_data(&path)?;
            let hit = data
                .prop_path(&["tool", "uv", "sources"])
                .and_then(|s| s.prop(&name));
            if hit.is_some_and(Tv::truthy) {
                out.push(pname.clone());
            }
        }
    }
    Ok(out)
}

struct Venv {
    /// Variables activation changed relative to the task environment.
    delta: Env,
    /// The task environment after activation.
    full: Env,
    /// The conditional `uv sync` of `installDependenciesIfNotExists`.
    install: Option<Step>,
}

/// `setVenvEnvironmentVariables`. A missing `PATH` becomes the string
/// `undefined`, as in the JS template literal.
fn set_venv(env: &mut Env, venv: &str) {
    let path = env.get("PATH").map_or("undefined", String::as_str);
    let path = format!("{venv}/bin:{path}");
    env.insert("VIRTUAL_ENV".into(), venv.to_string());
    env.insert("PATH".into(), path);
    env.remove("PYTHONHOME");
}

fn diff_env(base: &Env, env: &Env) -> Env {
    env.iter()
        .filter(|(k, v)| base.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn overlay(base: &Env, delta: &Env) -> Env {
    let mut env = base.clone();
    env.extend(delta.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

fn env_note(env: &Env) -> String {
    if env.is_empty() {
        return String::new();
    }
    let vars: Vec<String> = env
        .iter()
        .map(|(k, v)| format!("{k}={}", shell_quote(v)))
        .collect();
    format!(" [env: {}]", vars.join(" "))
}

/// Layer `delta` under a step's own env (the step's keys win).
fn layer_env(step: Step, delta: &Env) -> Step {
    if delta.is_empty() {
        return step;
    }
    match step {
        Step::Shell { script, cwd, env } => Step::Shell {
            script,
            cwd,
            env: overlay(delta, &env),
        },
        Step::Exec { argv, cwd, env } => Step::Exec {
            argv,
            cwd,
            env: overlay(delta, &env),
        },
        Step::Native { label, run } => {
            let delta = delta.clone();
            let label = format!("{label}{}", env_note(&delta));
            let wrapped: NativeFn = Arc::new(move |n: &NativeCtx<'_>| {
                let env = overlay(n.env, &delta);
                run(&NativeCtx {
                    workspace_root: n.workspace_root,
                    env: &env,
                })
            });
            Step::Native {
                label,
                run: wrapped,
            }
        }
    }
}

fn uv_step(args: Vec<String>, cwd: &str, env: &Env) -> Step {
    Step::Exec {
        argv: std::iter::once("uv".to_string()).chain(args).collect(),
        cwd: cwd.to_string(),
        env: env.clone(),
    }
}

/// `checkPrerequisites`: `command-exists uv`, run with the (possibly
/// activated) environment.
fn check_uv_step(delta: Env) -> Step {
    let label = format!(
        "check that `uv` is installed (command-exists uv){}",
        env_note(&delta)
    );
    let run: NativeFn = Arc::new(move |n: &NativeCtx<'_>| {
        let env = overlay(n.env, &delta);
        Ok(match check_uv(&env, n.workspace_root) {
            Ok(()) => StepOutput {
                success: true,
                output: String::new(),
            },
            Err(e) => StepOutput {
                success: false,
                output: error_line(&e),
            },
        })
    });
    Step::Native { label, run }
}

const UV_MISSING: &str = "UV is not installed. Please install UV before running this command.";

/// command-exists 1.2.9 on unix: a file `uv` in the cwd must be executable;
/// otherwise `command -v uv` in `/bin/sh` must print something.
fn check_uv(env: &Env, cwd: &Path) -> Result<()> {
    let local = cwd.join("uv");
    let found = if local.exists() {
        fs::metadata(&local).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    } else {
        Command::new("/bin/sh")
            .arg("-c")
            .arg("command -v uv 2>/dev/null && { echo >&1 uv; exit 0; }")
            .current_dir(cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    };
    if found {
        Ok(())
    } else {
        Err(eyre!(UV_MISSING))
    }
}

/// The `installDependenciesIfNotExists` half of `activateVenv`:
/// `provider.install(baseDir)` = check for uv, then `uv sync` in `baseDir`,
/// only when `<baseDir>/.venv` does not exist yet.
fn venv_install_step(base: String, shown: String, venv: String, delta: Env) -> Step {
    let label = format!(
        "if {shown}/.venv does not exist: check that `uv` is installed, then `uv sync` in \
         {shown}{}",
        env_note(&delta)
    );
    let run: NativeFn = Arc::new(move |n: &NativeCtx<'_>| {
        if Path::new(&venv).exists() {
            return Ok(StepOutput {
                success: true,
                output: String::new(),
            });
        }
        let env = overlay(n.env, &delta);
        let mut log = Log {
            silent: false,
            out: String::new(),
        };
        log.info(&format!(
            "\n  Creating virtual environment in  {base} ...\n"
        ));
        let result = check_uv(&env, n.workspace_root)
            .and_then(|()| run_uv(&mut log, &["sync"], &base, n.workspace_root, &env));
        Ok(log.finish(result))
    });
    Step::Native { label, run }
}

// ---------------------------------------------------------------------------
// build

/// `UVProvider.build` inputs, fixed at planning time.
struct BuildCfg {
    project_name: String,
    project_root: String,
    is_workspace: bool,
    ignore: Vec<IgnorePattern>,
    ignore_raw: Vec<String>,
    build_folder: Option<String>,
    locked_versions: bool,
    dev_dependencies: bool,
    /// `bundleLocalDependencies === true`.
    bundle_local: bool,
    output_path: String,
    format: Option<String>,
    skip_build: bool,
    keep_build_folder: bool,
    silent: bool,
    /// `(root, build target options)` per project.
    projects: Vec<(String, Option<JsonMap>)>,
}

impl BuildCfg {
    fn label(&self) -> String {
        let root = &self.project_root;
        let folder = self
            .build_folder
            .clone()
            .unwrap_or_else(|| "<os.tmpdir()>/nx-python/build/<uuid v4>".into());
        let resolver = if self.locked_versions {
            let lock_dir = if self.is_workspace {
                "."
            } else {
                root.as_str()
            };
            format!(
                "dependencies pinned from `uv export --format requirements-txt --no-hashes \
                 --no-header [--no-annotate if uv>=0.6.11] --frozen --no-emit-project --project \
                 {root}{}` (cwd .; `uv lock` in {root} first when {lock_dir}/uv.lock is missing), \
                 `-e`/`.` entries bundled as local packages, optional-dependencies pinned from \
                 {lock_dir}/uv.lock",
                if self.dev_dependencies {
                    ""
                } else {
                    " --no-dev"
                }
            )
        } else {
            let sources: BTreeMap<&str, Vec<String>> = self
                .projects
                .iter()
                .filter_map(|(r, o)| {
                    let o = o.as_ref()?;
                    let keys: Vec<String> = ["publish", "customSourceUrl", "customSourceName"]
                        .iter()
                        .filter_map(|k| o.get(*k).map(|v| format!("{k}={v}")))
                        .collect();
                    (!keys.is_empty()).then_some((r.as_str(), keys))
                })
                .collect();
            format!(
                "dependencies resolved from pyproject.toml (bundleLocalDependencies={}), local \
                 [tool.uv.sources] packages bundled or pinned to their version{}",
                self.bundle_local,
                if sources.is_empty() {
                    String::new()
                } else {
                    format!(" (build options: {sources:?})")
                }
            )
        };
        let build = if self.skip_build {
            "skipBuild: no `uv build`".to_string()
        } else {
            format!(
                "`uv build{}` in the build folder, then replace {} with its dist/",
                self.format
                    .as_deref()
                    .map(|f| format!(" --{f}"))
                    .unwrap_or_default(),
                self.output_path
            )
        };
        format!(
            "@nxlv/python:build {}: copy {root}/* except ignorePaths {:?} and __pycache__ into \
             {folder}; pyproject.toml: dependency-groups cleared, {resolver}; {build}; {}",
            self.project_name,
            self.ignore_raw,
            if self.keep_build_folder {
                "keep the build folder"
            } else {
                "remove the build folder"
            }
        )
    }
}

/// Build-step log: `logger.info` (hidden by `silent`) and `console.log`.
/// Errors are always shown.
struct Log {
    silent: bool,
    out: String,
}

impl Log {
    fn info(&mut self, msg: &str) {
        if !self.silent {
            self.console(msg);
        }
    }

    fn console(&mut self, msg: &str) {
        self.out.push_str(msg);
        self.out.push('\n');
    }

    fn finish(mut self, result: Result<()>) -> StepOutput {
        match result {
            Ok(()) => StepOutput {
                success: true,
                output: self.out,
            },
            Err(e) => {
                self.out.push_str(&error_line(&e));
                StepOutput {
                    success: false,
                    output: self.out,
                }
            }
        }
    }
}

fn error_line(e: &eyre::Report) -> String {
    format!("\n   ERROR  {e:#}\n\n")
}

/// `UVProvider.build`. A failure leaves the build folder behind, as the JS
/// does (it has no cleanup on error).
fn run_build(cfg: &BuildCfg, n: &NativeCtx<'_>) -> StepOutput {
    let mut log = Log {
        silent: cfg.silent,
        out: String::new(),
    };
    let result = build_in_folder(cfg, n, &mut log);
    log.finish(result)
}

fn build_in_folder(cfg: &BuildCfg, n: &NativeCtx<'_>, log: &mut Log) -> Result<()> {
    let ws_path = std::path::absolute(n.workspace_root)?;
    let ws = ws_string(&ws_path)?;
    let at = |p: &str| ws_path.join(p);
    log.info(&format!("\n  Building project  {} ...\n", cfg.project_name));
    let folder = match &cfg.build_folder {
        Some(f) => f.clone(),
        None => jspath::join(&[&os_tmpdir(n.env), "nx-python", "build", &uuid_v4()?]),
    };
    fs::create_dir_all(at(&folder))?;
    log.info("  Copying project files to a temporary folder");
    for file in read_dir_sorted(&at(&cfg.project_root))? {
        if !cfg.ignore.iter().any(|p| p.matches(&file)) {
            let source = jspath::join(&[&cfg.project_root, &file]);
            let target = jspath::join(&[&folder, &file]);
            copy_sync(&ws, &source, &target, true)?;
        }
    }
    let manifest = jspath::join(&[&folder, "pyproject.toml"]);
    let mut data = pyproject_data(&at(&manifest))?;
    data.set("dependency-groups", Tv::Table(Table::default()));
    let mut resolver = Resolver {
        cfg,
        ws: &ws,
        env: n.env,
        folder: &folder,
        log: &mut *log,
        root_lock: None,
    };
    if cfg.locked_versions {
        resolver.locked(&mut data)?;
    } else {
        resolver.log.info("  Resolving dependencies...");
        resolver.update_pyproject(&mut data, &mut Vec::new())?;
    }
    fs::write(at(&manifest), jstoml::stringify(&data)?)?;
    let dist = jspath::join(&[&folder, "dist"]);
    remove_sync(&at(&dist))?;
    if !cfg.skip_build {
        log.info("  Generating sdist and wheel artifacts");
        let format = cfg.format.as_deref().map(|f| format!("--{f}"));
        let mut args = vec!["build"];
        args.extend(format.as_deref());
        run_uv(log, &args, &folder, &ws_path, n.env)?;
        remove_sync(&at(&cfg.output_path))?;
        fs::create_dir_all(at(&cfg.output_path))?;
        log.info(&format!(
            "  Artifacts generated at {} folder",
            cfg.output_path
        ));
        copy_sync(&ws, &dist, &cfg.output_path, false)?;
    }
    if cfg.keep_build_folder {
        log.console(&format!("  Build folder kept at {folder}"));
    } else {
        remove_sync(&at(&folder))?;
    }
    Ok(())
}

/// Dependency resolution of a build: `LockedDependencyResolver` and
/// `ProjectDependencyResolver`.
struct Resolver<'a> {
    cfg: &'a BuildCfg,
    /// `context.root` = `process.cwd()` (absolute).
    ws: &'a str,
    env: &'a Env,
    folder: &'a str,
    log: &'a mut Log,
    root_lock: Option<Table>,
}

const TAB: &str = "    ";

impl Resolver<'_> {
    fn at(&self, p: &str) -> PathBuf {
        Path::new(self.ws).join(p)
    }

    // -- LockedDependencyResolver ------------------------------------------

    fn locked(&mut self, data: &mut Table) -> Result<()> {
        self.log.info("  Resolving dependencies...");
        let requirements = self.requirements_txt()?;
        let mut result = Vec::new();
        for line in requirements.split('\n') {
            if js_trim(line).is_empty() {
                continue;
            }
            if line.starts_with("-e") || line.starts_with('.') {
                let location = js_trim(&line.replacen("-e", "", 1)).to_string();
                let dep_path = self.local_path(&location);
                let dep_manifest = jspath::join(&[&dep_path, "pyproject.toml"]);
                if !self.at(&dep_manifest).exists() {
                    self.log.info(&format!(
                        "    • Skipping local dependency {dep_path} as pyproject.toml not found"
                    ));
                    continue;
                }
                let project_data = pyproject_data(&self.at(&dep_manifest))?;
                self.log.info(&format!(
                    "    • Adding {} local dependency",
                    project_name(&project_data)?
                ));
                self.include_dependency_package(&project_data, &dep_path, data)?;
                continue;
            }
            self.log
                .info(&format!("    • Adding {} dependency", js_trim(line)));
            result.push(Tv::Str(js_trim(line).to_string()));
        }
        let Some(Tv::Table(project)) = data.get_mut("project") else {
            bail!("pyproject.toml has no [project] table (Cannot set properties of undefined)");
        };
        project.set("dependencies", Tv::Arr(result));
        data.set("dependency-groups", Tv::Table(Table::default()));
        if data
            .prop_path(&["tool", "uv", "sources"])
            .is_some_and(Tv::truthy)
            && let Some(Tv::Table(uv)) = data.prop_path_mut(&["tool", "uv"])
        {
            uv.set("sources", Tv::Table(Table::default()));
        }

        let extras = match data.prop_path(&["project", "optional-dependencies"]) {
            None | Some(Tv::Undefined | Tv::Null) => Vec::new(),
            Some(Tv::Table(t)) => t.keys(),
            Some(other) => bail!("project.optional-dependencies is a {}", other.kind()),
        };
        if extras.is_empty() {
            return Ok(());
        }
        let lock_path = self.lock_path();
        if !self.at(&lock_path).exists() {
            bail!("uv.lock file not found");
        }
        let lock = uv_lockfile(&self.at(&lock_path))?
            .ok_or_else(|| eyre!("failed to get uv.lock file"))?;
        for extra in extras {
            let original = match data.prop_path(&["project", "optional-dependencies", &extra]) {
                Some(Tv::Arr(items)) => items.clone(),
                _ => bail!("project.optional-dependencies.{extra} is not an array"),
            };
            let locked: Vec<(Table, Vec<String>)> = original
                .iter()
                .filter_map(|dep| {
                    let dep = dep.js_str();
                    let name =
                        normalize_dependency_name(&dep).unwrap_or_else(|| "undefined".into());
                    match lock.get(&name) {
                        Some(Tv::Table(pkg)) => Some((pkg.clone(), extract_extras(Some(&dep)))),
                        _ => None,
                    }
                })
                .collect();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            if let Some(Tv::Arr(main)) = data.prop_path(&["project", "dependencies"]) {
                for dep in main {
                    seen.insert(
                        normalize_dependency_name(&dep.js_str())
                            .unwrap_or_else(|| "undefined".into()),
                    );
                }
            }
            let mut resolved = Vec::new();
            self.resolve_extras_tree(&lock, data, &locked, 1, &mut resolved, &mut seen)?;
            self.log.info(&format!(
                "{TAB}• Extra: {extra} - {} Locked Dependencies",
                resolved.join(", ")
            ));
            if let Some(Tv::Table(optional)) =
                data.prop_path_mut(&["project", "optional-dependencies"])
            {
                optional.set(&extra, Tv::Arr(resolved.into_iter().map(Tv::Str).collect()));
            }
        }
        Ok(())
    }

    fn lock_path(&self) -> String {
        if self.cfg.is_workspace {
            jspath::join(&[self.ws, "uv.lock"])
        } else {
            jspath::join(&[&self.cfg.project_root, "uv.lock"])
        }
    }

    /// A local path from the export or the lock: as-is at the workspace root,
    /// else relative to the project.
    fn local_path(&self, location: &str) -> String {
        if self.cfg.is_workspace {
            location.to_string()
        } else {
            let abs = jspath::resolve(self.ws, &[&self.cfg.project_root, location]);
            jspath::relative(self.ws, self.ws, &abs)
        }
    }

    /// `getProjectRequirementsTxt`.
    fn requirements_txt(&mut self) -> Result<String> {
        let version = uv_version(self.ws, self.env)?;
        let mut args = vec![
            "export",
            "--format",
            "requirements-txt",
            "--no-hashes",
            "--no-header",
        ];
        if semver_gte(&version, (0, 6, 11))? {
            args.push("--no-annotate");
        }
        args.extend([
            "--frozen",
            "--no-emit-project",
            "--project",
            &self.cfg.project_root,
        ]);
        if !self.cfg.dev_dependencies {
            args.push("--no-dev");
        }
        if !self.at(&self.lock_path()).exists() {
            self.log.info("  Generating uv.lock file");
            let (status, out) = run_merged(shell_command(
                "uv lock",
                &self.at(&self.cfg.project_root),
                self.env,
            ))?;
            self.log.out.push_str(&out);
            if !status.success() {
                bail!(
                    "failed to generate uv.lock file with exit code {}",
                    exit_code(status)
                );
            }
        }
        let output = shell_command(
            &format!("uv {}", args.join(" ")),
            Path::new(self.ws),
            self.env,
        )
        .stdin(Stdio::null())
        .output()?;
        if !output.status.success() {
            // The JS pipes and drops stderr; it is kept here for diagnosis.
            self.log
                .out
                .push_str(&String::from_utf8_lossy(&output.stderr));
            bail!(
                "failed to export requirements txt with exit code {}",
                exit_code(output.status)
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// `resolveExtrasLockedDependencyTree`.
    fn resolve_extras_tree(
        &mut self,
        lock: &Table,
        data: &mut Table,
        deps: &[(Table, Vec<String>)],
        level: usize,
        resolved: &mut Vec<String>,
        seen: &mut BTreeSet<String>,
    ) -> Result<()> {
        let tab = TAB.repeat(level);
        for (dep, extras) in deps {
            let name = dep
                .get("name")
                .map_or_else(|| "undefined".into(), Tv::js_str);
            self.log
                .info(&format!("{tab}• Resolving dependency: {name}"));
            let editable = match dep.get("source") {
                Some(Tv::Table(source)) => source
                    .get("editable")
                    .filter(|e| e.truthy())
                    .map(Tv::js_str),
                _ => bail!("uv.lock package {name} has no source"),
            };
            if let Some(editable) = &editable {
                let dep_path = self.local_path(editable);
                let manifest = jspath::join(&[&dep_path, "pyproject.toml"]);
                if self.at(&manifest).exists() {
                    let project_data = pyproject_data(&self.at(&manifest))?;
                    self.log.info(&format!(
                        "    • Adding {} local dependency",
                        project_name(&project_data)?
                    ));
                    self.include_dependency_package(&project_data, &dep_path, data)?;
                }
            }
            if editable.is_none() && !seen.contains(&name) {
                resolved.push(format!(
                    "{name}=={}",
                    dep.get("version")
                        .map_or_else(|| "undefined".into(), Tv::js_str)
                ));
                seen.insert(name.clone());
            }
            if let Some(Tv::Arr(children)) = dep.get("dependencies") {
                for child in children {
                    let child = child
                        .prop("name")
                        .map_or_else(|| "undefined".into(), Tv::js_str);
                    if seen.contains(&child) {
                        continue;
                    }
                    if let Some(Tv::Table(pkg)) = lock.get(&child)
                        && !pkg
                            .prop_path(&["source", "editable"])
                            .is_some_and(Tv::truthy)
                    {
                        resolved.push(format!(
                            "{child}=={}",
                            pkg.get("version")
                                .map_or_else(|| "undefined".into(), Tv::js_str)
                        ));
                        seen.insert(child);
                    }
                }
            }
            for extra in extras {
                let Some(Tv::Arr(entries)) =
                    dep.prop_path(&["optional-dependencies", extra.as_str()])
                else {
                    bail!(
                        "uv.lock package {name} has no optional-dependencies.{extra} (the JS fails \
                         with a TypeError reading its length)"
                    );
                };
                let pkg_deps: Vec<(Table, Vec<String>)> = entries
                    .iter()
                    .filter_map(|e| {
                        let n = e
                            .prop("name")
                            .map_or_else(|| "undefined".into(), Tv::js_str);
                        let Some(Tv::Table(pkg)) = lock.get(&n) else {
                            return None;
                        };
                        let extra = match e.prop("extra") {
                            Some(Tv::Arr(x)) => x.iter().map(Tv::js_str).collect(),
                            _ => Vec::new(),
                        };
                        Some((pkg.clone(), extra))
                    })
                    .collect();
                if !pkg_deps.is_empty() {
                    let mut names = Vec::new();
                    for (pkg, _) in &pkg_deps {
                        let n = pkg
                            .get("name")
                            .map_or_else(|| "undefined".into(), Tv::js_str);
                        if !seen.contains(&n)
                            && !pkg
                                .prop_path(&["source", "editable"])
                                .is_some_and(Tv::truthy)
                        {
                            resolved.push(format!(
                                "{n}=={}",
                                pkg.get("version")
                                    .map_or_else(|| "undefined".into(), Tv::js_str)
                            ));
                            seen.insert(n.clone());
                        }
                        names.push(n);
                    }
                    self.log.info(&format!(
                        "{tab}• Resolving extra: {extra} - {} Locked Dependencies",
                        names.join(", ")
                    ));
                }
                self.resolve_extras_tree(lock, data, &pkg_deps, level, resolved, seen)?;
            }
        }
        Ok(())
    }

    // -- includeDependencyPackage -----------------------------------------

    /// Copy a local dependency's packages into the build folder and register
    /// them with the target's build backend (hatchling or uv_build only).
    fn include_dependency_package(
        &mut self,
        project_data: &Table,
        dep_root: &str,
        data: &mut Table,
    ) -> Result<()> {
        let backend = match data.get("build-system") {
            Some(Tv::Table(bs)) => bs.get("build-backend").cloned().unwrap_or(Tv::Undefined),
            _ => bail!(
                "pyproject.toml has no [build-system] table (includeDependencyPackage reads it unguarded)"
            ),
        };
        let hatch = backend == Tv::Str("hatchling.build".into());
        let uv_build = backend == Tv::Str("uv_build".into());
        let target_src = self.at(&jspath::join(&[self.folder, "src"])).exists();
        let dep_src = jspath::join(&[self.ws, dep_root, "src"]);
        let is_src = self.at(&dep_src).exists();
        if hatch {
            let targets = ensure_table_path(data, &["tool", "hatch", "build", "targets"])?;
            if matches!(targets.get("wheel"), None | Some(Tv::Undefined | Tv::Null)) {
                let mut wheel = Table::default();
                wheel.set("packages", Tv::Arr(self.target_modules(target_src)?));
                targets.set("wheel", Tv::Table(wheel));
            }
        } else if uv_build {
            let backend_table = ensure_table_path(data, &["tool", "uv", "build-backend"])?;
            let modules = self.target_modules(false)?;
            backend_table.set("module-name", Tv::Arr(modules));
        } else {
            bail!(
                "Unsupported build system: {}, expected hatchling.build or uv_build",
                backend.js_str()
            );
        }
        let packages: Vec<String> = if is_src {
            read_dir_sorted(&self.at(&dep_src))?
        } else {
            match project_data
                .prop_path(&["tool", "hatch", "build", "targets", "wheel", "packages"])
            {
                None | Some(Tv::Undefined | Tv::Null) => Vec::new(),
                Some(Tv::Arr(items)) => items.iter().map(Tv::js_str).collect(),
                Some(other) => bail!(
                    "tool.hatch.build.targets.wheel.packages is a {}",
                    other.kind()
                ),
            }
        };
        for pkg in packages {
            let source = if is_src {
                jspath::join(&[&dep_src, &pkg])
            } else {
                jspath::join(&[self.ws, dep_root, &pkg])
            };
            let target = if target_src {
                jspath::join(&[self.folder, "src", &pkg])
            } else {
                jspath::join(&[self.folder, &pkg])
            };
            copy_sync(self.ws, &source, &target, true)?;
            // updateModules
            let (list, entry) = if hatch {
                let entry = if target_src {
                    format!("src/{pkg}")
                } else {
                    pkg.clone()
                };
                (
                    data.prop_path_mut(&["tool", "hatch", "build", "targets", "wheel", "packages"]),
                    entry,
                )
            } else {
                (
                    data.prop_path_mut(&["tool", "uv", "build-backend", "module-name"]),
                    pkg.clone(),
                )
            };
            let Some(Tv::Arr(list)) = list else {
                bail!("the build backend's package list is not an array (TypeError in the JS)");
            };
            if !list.iter().any(|v| v.strict_eq(&Tv::Str(entry.clone()))) {
                list.push(Tv::Str(entry));
            }
        }
        Ok(())
    }

    /// `getTargetModules`: the build folder's `src/` entries.
    fn target_modules(&self, full_path: bool) -> Result<Vec<Tv>> {
        let names = read_dir_sorted(&self.at(&jspath::join(&[self.folder, "src"])))?;
        Ok(names
            .into_iter()
            .map(|p| Tv::Str(if full_path { format!("src/{p}") } else { p }))
            .collect())
    }

    // -- ProjectDependencyResolver (lockedVersions: false) -----------------

    /// `updatePyproject`: one level of local dependencies per call, recursing
    /// while a bundled dependency brought in further local ones.
    fn update_pyproject(&mut self, py: &mut Table, logged: &mut Vec<String>) -> Result<()> {
        let mut more_levels = false;
        for (group, dependency_name) in project_dependencies(py)? {
            let normalized = normalize_dependency_name(&dependency_name);
            let dependency = find_dependency(py, &group, normalized.as_deref());
            let extras = extract_extras(dependency.as_deref());
            let Some(normalized) = normalized else {
                continue;
            };
            let dependency_shown = dependency.clone().unwrap_or_else(|| "undefined".into());
            let source = py
                .prop_path(&["tool", "uv", "sources", &normalized])
                .cloned();
            if !source.as_ref().is_some_and(Tv::truthy) {
                if !logged.contains(&dependency_shown) {
                    self.log
                        .info(&format!("{TAB}• Adding {dependency_shown} dependency"));
                    logged.push(dependency_shown);
                }
                continue;
            }
            let relative = source.as_ref().and_then(|s| s.prop("path")).cloned();
            let Some(dep_path) = self.dependency_path(&normalized, relative.as_ref())? else {
                continue;
            };
            let dep_manifest = jspath::join(&[&dep_path, "pyproject.toml"]);
            if !self.at(&dep_manifest).exists() {
                self.log.info(&format!(
                    "{TAB}• Skipping local dependency {dependency_shown} as pyproject.toml not found"
                ));
                continue;
            }
            let dep_py = pyproject_data(&self.at(&dep_manifest))?;
            let target_options = self.project_build_options(&dep_path)?;
            let publishable = target_options
                .as_ref()
                .and_then(|o| o.get("publish"))
                .cloned()
                .unwrap_or(Json::Bool(true));
            if self.cfg.bundle_local || publishable == Json::Bool(false) {
                if !logged.contains(&dependency_shown) {
                    self.log.info(&format!(
                        "{TAB}• Adding {dependency_shown} local dependency"
                    ));
                    logged.push(dependency_shown.clone());
                }
                self.include_dependency_package(&dep_py, &dep_path, py)?;
                let dep_deps = project_dependencies(&dep_py)?;
                remove_dependency(py, &group, dependency.as_deref())?;
                if let Some(Tv::Table(sources)) = py.prop_path_mut(&["tool", "uv", "sources"]) {
                    sources.remove(&normalized);
                }
                let dep_extras = match dep_py.prop_path(&["project", "optional-dependencies"]) {
                    Some(Tv::Table(t)) => t.clone(),
                    _ => Table::default(),
                };
                if dep_py
                    .prop_path(&["project", "optional-dependencies"])
                    .is_some_and(Tv::truthy)
                {
                    let mut target_extras =
                        match py.prop_path(&["project", "optional-dependencies"]) {
                            Some(Tv::Table(t)) => t.clone(),
                            _ => Table::default(),
                        };
                    for (extra_name, extra_data) in dep_extras.entries() {
                        if target_extras.get(extra_name).is_some_and(Tv::truthy) {
                            let Some(Tv::Arr(target)) = target_extras.get_mut(extra_name) else {
                                bail!("project.optional-dependencies.{extra_name} is not an array");
                            };
                            let Tv::Arr(extra_data) = extra_data else {
                                bail!(
                                    "optional-dependencies.{extra_name} of {dep_path} is not an array"
                                );
                            };
                            for dep in extra_data {
                                let normalized = normalize_dependency_name(&dep.js_str());
                                if normalized.is_some()
                                    && !target.iter().any(|d| {
                                        normalize_dependency_name(&d.js_str()) == normalized
                                    })
                                {
                                    target.push(dep.clone());
                                }
                            }
                        } else {
                            target_extras.set(extra_name, extra_data.clone());
                        }
                    }
                    project_table(py)?.set("optional-dependencies", Tv::Table(target_extras));
                }
                for (dep_group, dep_name) in dep_deps {
                    let mut group_to_use = dep_group.clone();
                    let normalized = normalize_dependency_name(&dep_name);
                    let dep_name_extras = extract_extras(Some(&dep_name));
                    if !extras.is_empty() && group == MAIN {
                        for extra_name in &extras {
                            let promote = match dep_extras.get(extra_name) {
                                Some(Tv::Arr(libs)) => libs
                                    .iter()
                                    .any(|l| normalize_dependency_name(&l.js_str()) == normalized),
                                _ => false,
                            };
                            if !promote {
                                continue;
                            }
                            group_to_use = MAIN.into();
                            let optional = py.prop_path_mut(&["project", "optional-dependencies"]);
                            if let Some(Tv::Table(optional)) = optional
                                && optional.get(extra_name).is_some_and(Tv::truthy)
                            {
                                let Some(Tv::Arr(list)) = optional.get(extra_name) else {
                                    bail!(
                                        "project.optional-dependencies.{extra_name} is not an array"
                                    );
                                };
                                let kept: Vec<Tv> = list
                                    .iter()
                                    .filter(|d| {
                                        normalize_dependency_name(&d.js_str()) != normalized
                                    })
                                    .cloned()
                                    .collect();
                                if kept.is_empty() {
                                    optional.remove(extra_name);
                                } else {
                                    optional.set(extra_name, Tv::Arr(kept));
                                }
                            }
                        }
                    }
                    let normalized_key = normalized.clone().unwrap_or_else(|| "undefined".into());
                    let dep_source = dep_py.prop_path(&["tool", "uv", "sources", &normalized_key]);
                    let nested_local = dep_source
                        .and_then(|s| s.prop("path"))
                        .is_some_and(Tv::truthy)
                        || dep_source
                            .and_then(|s| s.prop("workspace"))
                            .is_some_and(Tv::truthy);
                    if nested_local {
                        more_levels = true;
                        let existing = find_dependency(py, &group_to_use, normalized.as_deref());
                        let mut merged = extract_extras(existing.as_deref());
                        merged.extend(dep_name_extras);
                        let entry = if merged.is_empty() {
                            dep_name.clone()
                        } else {
                            format!("{normalized_key}[{}]", merged.join(","))
                        };
                        self.append_dependency(
                            py,
                            target_options.as_ref(),
                            &group_to_use,
                            &entry,
                            true,
                        )?;
                        let Some(Tv::Table(copied)) = dep_source.cloned() else {
                            bail!("tool.uv.sources.{normalized_key} of {dep_path} is not a table");
                        };
                        let sources = ensure_uv_sources(py)?;
                        sources.set(&normalized_key, Tv::Table(copied));
                        if let Some(Tv::Table(src)) = sources.get_mut(&normalized_key)
                            && let Some(path) =
                                src.get("path").filter(|p| p.truthy()).map(Tv::js_str)
                        {
                            let from = jspath::join(&[self.ws, &self.cfg.project_root]);
                            let to = jspath::resolve(self.ws, &[&dep_path, &path]);
                            src.set("path", Tv::Str(jspath::relative(self.ws, &from, &to)));
                        }
                    } else {
                        self.append_dependency(
                            py,
                            target_options.as_ref(),
                            &group_to_use,
                            &dep_name,
                            false,
                        )?;
                        if !logged.contains(&dep_name) {
                            self.log
                                .info(&format!("{TAB}• Adding {dep_name} dependency"));
                            logged.push(dep_name);
                        }
                    }
                }
            } else {
                // Publish mode: depend on the released version of the local
                // package (from its own index when it has one).
                let index = self.add_index(py, target_options.as_ref())?;
                let version = match dep_py.get("project") {
                    Some(Tv::Table(p)) => p
                        .get("version")
                        .map_or_else(|| "undefined".into(), Tv::js_str),
                    _ => bail!("{dep_manifest} has no [project] table"),
                };
                let dep_name = if version_range(&dependency_shown) {
                    dependency_shown.clone()
                } else {
                    format!("{dependency_shown}=={version}")
                };
                self.append_dependency(py, target_options.as_ref(), &group, &dep_name, true)?;
                let has_source = py
                    .prop_path(&["tool", "uv", "sources", &normalized])
                    .is_some_and(Tv::truthy);
                if has_source && !index.as_ref().is_some_and(Tv::truthy) {
                    if let Some(Tv::Table(sources)) = py.prop_path_mut(&["tool", "uv", "sources"]) {
                        sources.remove(&normalized);
                    }
                } else {
                    let mut entry = Table::default();
                    entry.set("index", index.unwrap_or(Tv::Undefined));
                    ensure_uv_sources(py)?.set(&normalized, Tv::Table(entry));
                }
                if !logged.contains(&dep_name) {
                    self.log
                        .info(&format!("{TAB}• Adding {dep_name} local dependency"));
                    logged.push(dep_name);
                }
            }
        }
        // Extras that are now main dependencies are no longer optional.
        let main: Vec<Option<String>> = match py.prop_path(&["project", "dependencies"]) {
            Some(Tv::Arr(items)) => items
                .iter()
                .map(|d| normalize_dependency_name(&d.js_str()))
                .collect(),
            _ => bail!("project.dependencies is not an array"),
        };
        if let Some(Tv::Table(optional)) = py.prop_path_mut(&["project", "optional-dependencies"]) {
            for name in optional.keys() {
                let Some(Tv::Arr(list)) = optional.get(&name) else {
                    bail!(
                        "project.optional-dependencies.{name} is not an array (TypeError in the JS)"
                    );
                };
                let kept: Vec<Tv> = list
                    .iter()
                    .filter(|d| !main.contains(&normalize_dependency_name(&d.js_str())))
                    .cloned()
                    .collect();
                if kept.is_empty() {
                    optional.remove(&name);
                } else {
                    optional.set(&name, Tv::Arr(kept));
                }
            }
        }
        if more_levels {
            // One level per call (`loggedDependencies` carries over).
            self.update_pyproject(py, logged)?;
        }
        Ok(())
    }

    /// `getDependencyPath`: the lock's editable path in a workspace, else the
    /// source path relative to the workspace root.
    fn dependency_path(
        &mut self,
        dependency: &str,
        relative: Option<&Tv>,
    ) -> Result<Option<String>> {
        let path = if self.cfg.is_workspace {
            if self.root_lock.is_none() {
                let lock = uv_lockfile(&self.at(&jspath::join(&[self.ws, "uv.lock"])))?
                    .ok_or_else(|| {
                        eyre!("the root uv.lock has no packages (TypeError in the JS)")
                    })?;
                self.root_lock = Some(lock);
            }
            self.root_lock
                .as_ref()
                .and_then(|l| l.get(dependency))
                .and_then(|p| p.prop("source"))
                .and_then(|s| s.prop("editable"))
                .filter(|e| e.truthy())
                .map(Tv::js_str)
        } else {
            match relative.filter(|r| r.truthy()) {
                Some(r) => {
                    let abs = jspath::resolve(self.ws, &[&self.cfg.project_root, &r.js_str()]);
                    Some(jspath::relative(self.ws, self.ws, &abs))
                }
                None => None,
            }
        };
        Ok(path.filter(|p| !p.is_empty()))
    }

    /// `getProjectConfig(root).targets?.build?.options`.
    fn project_build_options(&self, root: &str) -> Result<Option<JsonMap>> {
        let wanted = jspath::normalize(root);
        self.cfg
            .projects
            .iter()
            .find(|(r, _)| jspath::normalize(r) == wanted)
            .map(|(_, o)| o.clone())
            .ok_or_else(|| eyre!("Could not find project config for {root}"))
    }

    fn append_dependency(
        &mut self,
        py: &mut Table,
        target_options: Option<&JsonMap>,
        group: &str,
        dependency: &str,
        force: bool,
    ) -> Result<()> {
        let index = self.add_index(py, target_options)?;
        let normalized = normalize_dependency_name(dependency);
        let project = project_table(py)?;
        let list = if group == MAIN {
            match project.get_mut("dependencies") {
                Some(Tv::Arr(l)) => l,
                _ => bail!("project.dependencies is not an array"),
            }
        } else {
            let optional = ensure_table(project, "optional-dependencies")?;
            if matches!(optional.get(group), None | Some(Tv::Undefined | Tv::Null)) {
                optional.set(group, Tv::Arr(Vec::new()));
            }
            match optional.get_mut(group) {
                Some(Tv::Arr(l)) => l,
                _ => bail!("project.optional-dependencies.{group} is not an array"),
            }
        };
        match list
            .iter()
            .position(|d| normalize_dependency_name(&d.js_str()) == normalized)
        {
            Some(_) if !force => return Ok(()),
            Some(i) => list[i] = Tv::Str(dependency.to_string()),
            None => list.push(Tv::Str(dependency.to_string())),
        }
        if let Some(index) = index.filter(Tv::truthy) {
            let mut entry = Table::default();
            entry.set("index", index);
            let key = normalized.unwrap_or_else(|| "undefined".into());
            ensure_uv_sources(py)?.set(&key, Tv::Table(entry));
        }
        Ok(())
    }

    /// `addIndex`: register the dependency's `customSourceUrl` under
    /// `[[tool.uv.index]]`, renaming a clashing name with the URL's MD5.
    fn add_index(
        &mut self,
        py: &mut Table,
        target_options: Option<&JsonMap>,
    ) -> Result<Option<Tv>> {
        let Some(opts) = target_options else {
            return Ok(None);
        };
        let url = opts
            .get("customSourceUrl")
            .map(json_to_tv)
            .unwrap_or(Tv::Undefined);
        if !url.truthy() {
            return Ok(None);
        }
        let name = opts
            .get("customSourceName")
            .map(json_to_tv)
            .unwrap_or(Tv::Undefined);
        let Some(Tv::Table(uv)) = py.prop_path_mut(&["tool", "uv"]) else {
            bail!("pyproject.toml has no [tool.uv] table (addIndex reads it unguarded)");
        };
        let new_entry = |name: Tv| {
            let mut t = Table::default();
            t.set("name", name);
            t.set("url", url.clone());
            Tv::Table(t)
        };
        let indexes = uv.get("index").cloned();
        let (indexes, index_name) = match indexes.filter(Tv::truthy) {
            None => (vec![new_entry(name.clone())], name),
            Some(Tv::Arr(list)) => {
                let existing = list
                    .iter()
                    .find(|s| s.prop("name").unwrap_or(&Tv::Undefined).strict_eq(&name));
                match existing {
                    Some(e) if e.prop("url").unwrap_or(&Tv::Undefined).strict_eq(&url) => {
                        (list, name)
                    }
                    Some(_) => {
                        let Tv::Str(u) = &url else {
                            bail!("customSourceUrl must be a string")
                        };
                        let renamed =
                            Tv::Str(format!("{}-{}", name.js_str(), md5_hex(u.as_bytes())));
                        self.log.info(&format!(
                            "  Duplicate index for {} renamed to {}",
                            name.js_str(),
                            renamed.js_str()
                        ));
                        if list
                            .iter()
                            .any(|s| s.prop("name").unwrap_or(&Tv::Undefined).strict_eq(&renamed))
                        {
                            (list, renamed)
                        } else {
                            let mut list = list;
                            list.push(new_entry(renamed.clone()));
                            (list, renamed)
                        }
                    }
                    None => {
                        let mut list = list;
                        list.push(new_entry(name.clone()));
                        (list, name)
                    }
                }
            }
            Some(other) => bail!("tool.uv.index is a {}", other.kind()),
        };
        uv.set("index", Tv::Arr(indexes));
        Ok(Some(index_name))
    }
}

const MAIN: &str = "__main__";

fn project_table(py: &mut Table) -> Result<&mut Table> {
    match py.get_mut("project") {
        Some(Tv::Table(p)) => Ok(p),
        _ => bail!("pyproject.toml has no [project] table"),
    }
}

/// `getProjectDependencies`: `[group, dependency]` pairs, main first.
fn project_dependencies(py: &Table) -> Result<Vec<(String, String)>> {
    let Some(Tv::Arr(main)) = py.prop_path(&["project", "dependencies"]) else {
        bail!("pyproject.toml has no project.dependencies array (TypeError in the JS)");
    };
    let mut out: Vec<(String, String)> = main
        .iter()
        .map(|d| (MAIN.to_string(), d.js_str()))
        .collect();
    match py.prop_path(&["project", "optional-dependencies"]) {
        None | Some(Tv::Undefined | Tv::Null) => {}
        Some(Tv::Table(optional)) => {
            for (extra, deps) in optional.entries() {
                let Tv::Arr(deps) = deps else {
                    bail!("project.optional-dependencies.{extra} is not an array");
                };
                out.extend(deps.iter().map(|d| (extra.to_string(), d.js_str())));
            }
        }
        Some(other) => bail!("project.optional-dependencies is a {}", other.kind()),
    }
    Ok(out)
}

/// The group's first entry with the given normalized name.
fn find_dependency(py: &Table, group: &str, normalized: Option<&str>) -> Option<String> {
    let list = if group == MAIN {
        py.prop_path(&["project", "dependencies"])
    } else {
        py.prop_path(&["project", "optional-dependencies", group])
    };
    match list {
        Some(Tv::Arr(items)) => items
            .iter()
            .find(|d| normalize_dependency_name(&d.js_str()).as_deref() == normalized)
            .map(Tv::js_str),
        _ => None,
    }
}

/// `removeDependency`.
fn remove_dependency(py: &mut Table, group: &str, dependency: Option<&str>) -> Result<()> {
    let keep = |d: &Tv| !matches!((d, dependency), (Tv::Str(s), Some(dep)) if s == dep);
    let project = project_table(py)?;
    if group == MAIN {
        if let Some(Tv::Arr(list)) = project.get_mut("dependencies") {
            list.retain(keep);
        }
    } else {
        let optional = ensure_table(project, "optional-dependencies")?;
        let filtered = match optional.get(group) {
            Some(Tv::Arr(list)) => Tv::Arr(list.iter().filter(|d| keep(d)).cloned().collect()),
            _ => Tv::Undefined,
        };
        optional.set(group, filtered);
    }
    Ok(())
}

fn ensure_uv_sources(py: &mut Table) -> Result<&mut Table> {
    ensure_table_path(py, &["tool", "uv", "sources"])
}

/// JS `a.b ??= {}` down a path: missing (or null) tables are created,
/// anything else in the way is an error (assigning a property on a primitive
/// throws in strict mode).
fn ensure_table_path<'t>(t: &'t mut Table, path: &[&str]) -> Result<&'t mut Table> {
    let mut cur = t;
    for key in path {
        cur = ensure_table(cur, key)?;
    }
    Ok(cur)
}

fn ensure_table<'t>(t: &'t mut Table, key: &str) -> Result<&'t mut Table> {
    match t.get(key) {
        None | Some(Tv::Undefined | Tv::Null) => t.set(key, Tv::Table(Table::default())),
        Some(Tv::Table(_)) => {}
        Some(other) => bail!("`{key}` is a {}, not a table", other.kind()),
    }
    match t.get_mut(key) {
        Some(Tv::Table(inner)) => Ok(inner),
        _ => bail!("`{key}` is not a table"),
    }
}

fn project_name(data: &Table) -> Result<String> {
    match data.get("project") {
        Some(Tv::Table(p)) => Ok(p.get("name").map_or_else(|| "undefined".into(), Tv::js_str)),
        _ => bail!("a local dependency's pyproject.toml has no [project] table"),
    }
}

// ---------------------------------------------------------------------------
// Manifests, locks and processes.

/// `getPyprojectData`: missing or blank file = empty document.
fn pyproject_data(path: &Path) -> Result<Table> {
    if !path.exists() {
        return Ok(Table::default());
    }
    let src = fs::read_to_string(path).map_err(|e| eyre!("{}: {e}", path.display()))?;
    if js_trim(&src).is_empty() {
        return Ok(Table::default());
    }
    jstoml::parse(&src).map_err(|e| eyre!("{}: {e}", path.display()))
}

fn parse_toml_file(path: &Path) -> Result<Table> {
    let src = fs::read_to_string(path).map_err(|e| eyre!("{}: {e}", path.display()))?;
    jstoml::parse(&src).map_err(|e| eyre!("{}: {e}", path.display()))
}

/// `getUvLockfile`: packages keyed by name (a later duplicate replaces the
/// earlier value in place), with `requires-dist` and each `requires-dev`
/// group re-keyed by requirement name.
fn uv_lockfile(path: &Path) -> Result<Option<Table>> {
    if !path.exists() {
        return Ok(None);
    }
    let data = parse_toml_file(path)?;
    let packages = match data.get("package") {
        None | Some(Tv::Undefined | Tv::Null) => return Ok(None),
        Some(Tv::Arr(p)) => p,
        Some(other) => bail!("{}: `package` is a {}", path.display(), other.kind()),
    };
    let by_name = |list: Option<&Tv>| -> Result<Table> {
        let mut out = Table::default();
        match list {
            None | Some(Tv::Undefined | Tv::Null) => {}
            Some(Tv::Arr(items)) => {
                for req in items {
                    let name = req
                        .prop("name")
                        .map_or_else(|| "undefined".into(), Tv::js_str);
                    out.set(&name, req.clone());
                }
            }
            Some(other) => bail!("uv.lock requirement list is a {}", other.kind()),
        }
        Ok(out)
    };
    let mut out = Table::default();
    for pkg in packages {
        let Tv::Table(pkg) = pkg else {
            bail!("{}: a package is not a table", path.display())
        };
        let mut metadata = match pkg.get("metadata") {
            Some(Tv::Table(m)) => m.clone(),
            _ => Table::default(),
        };
        let dist = by_name(metadata.get("requires-dist"))?;
        let mut dev = Table::default();
        if let Some(Tv::Table(groups)) = metadata.get("requires-dev") {
            for (group, list) in groups.entries() {
                dev.set(group, Tv::Table(by_name(Some(list))?));
            }
        }
        metadata.set("requires-dist", Tv::Table(dist));
        metadata.set("requires-dev", Tv::Table(dev));
        let mut pkg = pkg.clone();
        pkg.set("metadata", Tv::Table(metadata));
        let name = pkg
            .get("name")
            .map_or_else(|| "undefined".into(), Tv::js_str);
        out.set(&name, Tv::Table(pkg));
    }
    Ok(Some(out))
}

/// `getUvVersion`: second word of `uv --version`.
fn uv_version(ws: &str, env: &Env) -> Result<String> {
    let output = Command::new("uv")
        .arg("--version")
        .current_dir(ws)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| eyre!("`uv --version` failed: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(js_trim(&text)
        .split(' ')
        .nth(1)
        .unwrap_or("undefined")
        .to_string())
}

/// semver 7 `gte(version, floor)` for a release floor; an unparseable
/// version throws like `new SemVer`.
fn semver_gte(version: &str, floor: (u64, u64, u64)) -> Result<bool> {
    let invalid = || eyre!("Invalid Version: {version}");
    let v = version.trim();
    let v = v.strip_prefix('v').unwrap_or(v);
    let v = v.split_once('+').map_or(v, |(core, _)| core);
    let (core, pre) = match v.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (v, None),
    };
    let nums = core
        .split('.')
        .map(|n| {
            let numeric = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
            if numeric && (n == "0" || !n.starts_with('0')) {
                n.parse::<u64>().map_err(|_| invalid())
            } else {
                Err(invalid())
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let [a, b, c] = nums[..] else {
        return Err(invalid());
    };
    if pre.is_some_and(str::is_empty) {
        return Err(invalid());
    }
    Ok((a, b, c) > floor || ((a, b, c) == floor && pre.is_none()))
}

/// `runUv`: logs the command (`console.log`), runs uv without a shell,
/// fails on a non-zero exit.
fn run_uv(log: &mut Log, args: &[&str], cwd: &str, ws: &Path, env: &Env) -> Result<()> {
    let command = format!("uv {}", args.join(" "));
    let at = if !cwd.is_empty() && cwd != "." {
        format!("at {cwd} folder")
    } else {
        String::new()
    };
    log.console(&format!("Running command: {command} {at}\n"));
    let mut cmd = Command::new("uv");
    cmd.args(args)
        .current_dir(ws.join(cwd))
        .env_clear()
        .envs(env);
    let (status, out) = run_merged(cmd)?;
    log.out.push_str(&out);
    if !status.success() {
        bail!(
            "{command} command failed with exit code {}",
            exit_code(status)
        );
    }
    Ok(())
}

/// `cross-spawn` with `shell: true`: `/bin/sh -c` over the space-joined
/// words.
fn shell_command(script: &str, cwd: &Path, env: &Env) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(script)
        .current_dir(cwd)
        .env_clear()
        .envs(env);
    cmd
}

/// Run with stdout and stderr on one channel (the order a terminal shows).
fn run_merged(mut cmd: Command) -> Result<(ExitStatus, String)> {
    let (mut reader, writer) = std::os::unix::net::UnixStream::pair()?;
    let stderr = writer.try_clone()?;
    cmd.stdout(Stdio::from(OwnedFd::from(writer)))
        .stderr(Stdio::from(OwnedFd::from(stderr)));
    let mut child = cmd.spawn()?;
    // The command keeps the write ends alive until dropped.
    drop(cmd);
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    let status = child.wait()?;
    Ok((status, String::from_utf8_lossy(&out).into_owned()))
}

/// JS `result.status`: `null` when a signal ended the process.
fn exit_code(status: ExitStatus) -> String {
    status
        .code()
        .map_or_else(|| "null".into(), |c| c.to_string())
}

/// Node `os.tmpdir()` on POSIX.
fn os_tmpdir(env: &Env) -> String {
    let dir = ["TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|k| env.get(*k).filter(|v| !v.is_empty()))
        .map_or("/tmp", String::as_str);
    if dir.len() > 1 && dir.ends_with('/') {
        dir[..dir.len() - 1].to_string()
    } else {
        dir.to_string()
    }
}

fn uuid_v4() -> Result<String> {
    let mut b = [0u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn ws_string(ws: &Path) -> Result<String> {
    let abs = std::path::absolute(ws)?;
    abs.to_str()
        .map(str::to_string)
        .ok_or_else(|| eyre!("workspace root {} is not UTF-8", abs.display()))
}

/// `fs.readdirSync`: names sorted bytewise (libuv's scandir sorts).
fn read_dir_sorted(dir: &Path) -> Result<Vec<String>> {
    let entries = fs::read_dir(dir).map_err(|e| eyre!("{e}, scandir '{}'", dir.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        names.push(
            name.into_string()
                .map_err(|n| eyre!("{}: non-UTF-8 file name {n:?}", dir.display()))?,
        );
    }
    names.sort();
    Ok(names)
}

/// `pycacheFilter`: skip anything under a `__pycache__` segment of the
/// source path as the JS spells it.
fn pycache_filter(src: &str) -> bool {
    !src.split('/').any(|seg| seg == "__pycache__")
}

/// `fs.rmSync(p, {recursive: true, force: true})`.
fn remove_sync(p: &Path) -> Result<()> {
    match fs::symlink_metadata(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
        Ok(m) if m.is_dir() => Ok(fs::remove_dir_all(p)?),
        Ok(_) => Ok(fs::remove_file(p)?),
    }
}

/// fs-extra 11 `copySync(src, dest, {filter?})` (overwrite on, no
/// dereference, no timestamps). Paths are JS strings relative to the
/// workspace root (or absolute); the filter sees them as spelled.
fn copy_sync(ws: &str, src: &str, dest: &str, filter: bool) -> Result<()> {
    if filter && !pycache_filter(src) {
        return Ok(());
    }
    let (s, d) = (Path::new(ws).join(src), Path::new(ws).join(dest));
    let meta = fs::symlink_metadata(&s).map_err(|e| eyre!("{e}, lstat '{src}'"))?;
    let dest_meta = lstat_opt(&d)?;
    check_paths(ws, src, dest, &meta, dest_meta.as_ref())?;
    if let Some(parent) = d.parent() {
        fs::create_dir_all(parent)?;
    }
    copy_item(ws, src, dest, &meta, dest_meta.as_ref(), filter)
}

fn lstat_opt(p: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(p) {
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// fs-extra `checkPathsSync`.
fn check_paths(
    ws: &str,
    src: &str,
    dest: &str,
    meta: &fs::Metadata,
    dest_meta: Option<&fs::Metadata>,
) -> Result<()> {
    if let Some(dm) = dest_meta {
        if dm.ino() == meta.ino() && dm.dev() == meta.dev() {
            bail!("Source and destination must not be the same.");
        }
        if meta.is_dir() && !dm.is_dir() {
            bail!("Cannot overwrite non-directory '{dest}' with directory '{src}'.");
        }
        if !meta.is_dir() && dm.is_dir() {
            bail!("Cannot overwrite directory '{dest}' with non-directory '{src}'.");
        }
    }
    if meta.is_dir() && is_src_subdir(ws, src, dest) {
        bail!("Cannot copy '{src}' to a subdirectory of itself, '{dest}'.");
    }
    Ok(())
}

fn is_src_subdir(ws: &str, src: &str, dest: &str) -> bool {
    let s = jspath::resolve(ws, &[src]);
    let d = jspath::resolve(ws, &[dest]);
    let d: Vec<&str> = d.split('/').filter(|x| !x.is_empty()).collect();
    s.split('/')
        .filter(|x| !x.is_empty())
        .enumerate()
        .all(|(i, seg)| d.get(i) == Some(&seg))
}

fn copy_item(
    ws: &str,
    src: &str,
    dest: &str,
    meta: &fs::Metadata,
    dest_meta: Option<&fs::Metadata>,
    filter: bool,
) -> Result<()> {
    let (s, d) = (Path::new(ws).join(src), Path::new(ws).join(dest));
    let ft = meta.file_type();
    if ft.is_dir() {
        if dest_meta.is_none() {
            fs::create_dir(&d)?;
        }
        for entry in fs::read_dir(&s)? {
            let name = entry?.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| eyre!("{src}: non-UTF-8 file name {name:?}"))?;
            let (src_item, dest_item) = (jspath::join(&[src, name]), jspath::join(&[dest, name]));
            if filter && !pycache_filter(&src_item) {
                continue;
            }
            let item_meta = fs::symlink_metadata(Path::new(ws).join(&src_item))?;
            let item_dest = lstat_opt(&Path::new(ws).join(&dest_item))?;
            check_paths(ws, &src_item, &dest_item, &item_meta, item_dest.as_ref())?;
            copy_item(
                ws,
                &src_item,
                &dest_item,
                &item_meta,
                item_dest.as_ref(),
                filter,
            )?;
        }
        if dest_meta.is_none() {
            fs::set_permissions(&d, meta.permissions())?;
        }
    } else if ft.is_file() || ft.is_char_device() || ft.is_block_device() {
        if dest_meta.is_some() {
            fs::remove_file(&d)?;
        }
        fs::copy(&s, &d)?;
        fs::set_permissions(&d, meta.permissions())?;
    } else if ft.is_symlink() {
        let target = fs::read_link(&s)?;
        if dest_meta.is_some() {
            fs::remove_file(&d)?;
        }
        std::os::unix::fs::symlink(&target, &d)?;
    } else if ft.is_socket() {
        bail!("Cannot copy a socket file: {dest}");
    } else if ft.is_fifo() {
        bail!("Cannot copy a FIFO pipe: {dest}");
    } else {
        bail!("Unknown file: {src}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Small JS semantics.

/// JS truthiness of an option value.
fn truthy(v: Option<&Json>) -> bool {
    match v {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(Json::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Json::String(s)) => !s.is_empty(),
        Some(Json::Array(_) | Json::Object(_)) => true,
    }
}

/// `[].concat(value)`: an array spreads, anything else is one element;
/// `Array.join` renders null as empty.
fn js_concat(v: Option<&Json>) -> Vec<String> {
    let word = |v: &Json| {
        if v.is_null() {
            String::new()
        } else {
            js_string(v)
        }
    };
    match v {
        None => Vec::new(),
        Some(Json::Array(items)) => items.iter().map(word).collect(),
        Some(other) => vec![word(other)],
    }
}

/// `(options.args ?? '').split(' ').filter((arg) => !!arg)`.
fn split_args(v: Option<&Json>) -> Vec<String> {
    match v {
        None | Some(Json::Null) => Vec::new(),
        Some(a) => js_string(a)
            .split(' ')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

fn push_verbosity(args: &mut Vec<String>, opts: &JsonMap) {
    if truthy(opts.get("verbose")) {
        args.push("-v".into());
    } else if truthy(opts.get("debug")) {
        args.push("-vvv".into());
    }
}

fn push_cache_dir(args: &mut Vec<String>, opts: &JsonMap) {
    if let Some(dir) = opts.get("cacheDir").filter(|d| truthy(Some(*d))) {
        args.push("--cache-dir".into());
        args.push(js_string(dir));
    }
}

fn required_string(opts: &JsonMap, key: &str, task: &str) -> Result<String> {
    match opts.get(key) {
        None | Some(Json::Null) => bail!("{task}: option `{key}` is required"),
        Some(v) => Ok(js_string(v)),
    }
}

/// `extractBooleanFlag`: `--flag` → true; `--flag true|false` consumes the
/// value too. The value test trims, the result compares untrimmed.
fn extract_boolean_flag(unparsed: &mut Vec<String>, flag: &str) -> Option<bool> {
    let index = unparsed.iter().position(|a| js_trim(a) == flag)?;
    let next = unparsed
        .get(index + 1)
        .map(|n| js_trim(&n.to_lowercase()).to_string());
    if matches!(next.as_deref(), Some("true" | "false")) {
        let removed: Vec<String> = unparsed.drain(index..index + 2).collect();
        return Some(removed[1].to_lowercase() == "true");
    }
    unparsed.remove(index);
    Some(true)
}

/// ECMAScript `String.prototype.trim` whitespace.
fn js_is_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(js_is_space)
}

/// `normalizeDependencyName`: the leading `[a-zA-Z0-9-_]+` run.
fn normalize_dependency_name(dep: &str) -> Option<String> {
    let end = dep
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(dep.len());
    (end > 0).then(|| dep[..end].to_string())
}

/// `extractExtraFromDependencyName`: `/\[(.*)\]/` (greedy, no line
/// terminators), split on commas, trimmed.
fn extract_extras(dep: Option<&str>) -> Vec<String> {
    let Some(dep) = dep.filter(|d| !d.is_empty()) else {
        return Vec::new();
    };
    let is_terminator = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    for (open, _) in dep.match_indices('[') {
        let rest = &dep[open + 1..];
        let line = rest.find(is_terminator).map_or(rest, |end| &rest[..end]);
        if let Some(close) = line.rfind(']') {
            return line[..close]
                .split(',')
                .map(|e| js_trim(e).to_string())
                .collect();
        }
    }
    Vec::new()
}

/// `VERSION_RANGE_REGEX` of the project resolver: a name followed by one or
/// more comparison clauses and nothing else.
fn version_range(dep: &str) -> bool {
    const OPS: [&str; 8] = ["==", "!=", "~=", "===", ">=", "<=", ">", "<"];
    let s: Vec<char> = dep.chars().collect();
    let ws = |mut i: usize| {
        while i < s.len() && js_is_space(s[i]) {
            i += 1;
        }
        i
    };
    fn clause(s: &[char], i: usize, ws: &dyn Fn(usize) -> usize) -> bool {
        OPS.iter().any(|op| {
            let op: Vec<char> = op.chars().collect();
            if !s[i..].starts_with(&op) {
                return false;
            }
            let start = ws(i + op.len());
            let mut end = start;
            while end < s.len() && !js_is_space(s[end]) && s[end] != ',' && s[end] != ';' {
                end += 1;
            }
            (start + 1..=end).rev().any(|stop| rest(s, stop, ws))
        })
    }
    fn rest(s: &[char], i: usize, ws: &dyn Fn(usize) -> usize) -> bool {
        if i == s.len() {
            return true;
        }
        let with_comma = s[i] == ',' && clause(s, ws(i + 1), ws);
        with_comma || clause(s, ws(i), ws)
    }
    let name_end = s
        .iter()
        .position(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
        .unwrap_or(s.len());
    name_end > 0 && clause(&s, ws(name_end), &ws)
}

fn json_to_tv(v: &Json) -> Tv {
    match v {
        Json::Null => Tv::Null,
        Json::Bool(b) => Tv::Bool(*b),
        Json::Number(n) => n
            .as_i64()
            .map_or_else(|| Tv::Float(n.as_f64().unwrap_or(f64::NAN)), Tv::Int),
        Json::String(s) => Tv::Str(s.clone()),
        Json::Array(items) => Tv::Arr(items.iter().map(json_to_tv).collect()),
        Json::Object(map) => {
            let mut t = Table::default();
            for (k, v) in map {
                t.set(k, json_to_tv(v));
            }
            Tv::Table(t)
        }
    }
}

/// minimatch 10 `minimatch(name, pattern, {dot: true})` against one path
/// segment. Negation, comments, `*`/`?`/classes/`{a,b}`/`**` are ported;
/// extglobs, POSIX classes and numeric brace ranges are refused.
struct IgnorePattern {
    negate: bool,
    matcher: Option<GlobMatcher>,
}

impl IgnorePattern {
    fn new(pattern: &str) -> Result<IgnorePattern> {
        if pattern.starts_with('#') {
            return Ok(IgnorePattern {
                negate: false,
                matcher: None,
            });
        }
        let bangs = pattern.len() - pattern.trim_start_matches('!').len();
        let body = &pattern[bangs..];
        let extglob = ["@(", "+(", "*(", "?(", "!("]
            .iter()
            .any(|x| body.contains(x));
        let range = body.contains('{') && body.contains("..");
        if extglob || body.contains("[:") || range {
            bail!(
                "pattern `{pattern}` uses minimatch syntax butler does not port (extglob, POSIX class or brace range)"
            );
        }
        let matcher = if body.is_empty() {
            None
        } else {
            let glob = GlobBuilder::new(body)
                .literal_separator(true)
                .backslash_escape(true)
                .build()
                .map_err(|e| eyre!("pattern `{pattern}`: {e}"))?;
            Some(glob.compile_matcher())
        };
        Ok(IgnorePattern {
            negate: bangs % 2 == 1,
            matcher,
        })
    }

    fn matches(&self, name: &str) -> bool {
        let hit = self.matcher.as_ref().is_some_and(|m| m.is_match(name));
        hit != self.negate
    }
}

/// RFC 1321 MD5 (Node `createHash('md5')`), for `resolveDuplicateIndexes`.
fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_le_bytes());
    for chunk in msg.chunks_exact(64) {
        let m: Vec<u32> = chunk
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    state
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .map(|x| format!("{x:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// Node `path.posix`, on strings (paths appear in manifests and messages).

mod jspath {
    /// `path.normalize`.
    pub fn normalize(p: &str) -> String {
        if p.is_empty() {
            return ".".into();
        }
        let absolute = p.starts_with('/');
        let trailing = p.ends_with('/');
        let mut out: Vec<&str> = Vec::new();
        for seg in p.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if out.last().is_some_and(|l| *l != "..") {
                        out.pop();
                    } else if !absolute {
                        out.push("..");
                    }
                }
                s => out.push(s),
            }
        }
        let mut s = out.join("/");
        if s.is_empty() && !absolute {
            s.push('.');
        }
        if !s.is_empty() && trailing {
            s.push('/');
        }
        if absolute { format!("/{s}") } else { s }
    }

    /// `path.join`.
    pub fn join(parts: &[&str]) -> String {
        let joined = parts
            .iter()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("/");
        if joined.is_empty() {
            ".".into()
        } else {
            normalize(&joined)
        }
    }

    /// `path.resolve` with `cwd` as `process.cwd()` (absolute).
    pub fn resolve(cwd: &str, parts: &[&str]) -> String {
        let mut acc = String::new();
        for p in parts.iter().rev().filter(|p| !p.is_empty()) {
            acc = if acc.is_empty() {
                (*p).to_string()
            } else {
                format!("{p}/{acc}")
            };
            if p.starts_with('/') {
                break;
            }
        }
        if !acc.starts_with('/') {
            acc = if acc.is_empty() {
                cwd.to_string()
            } else {
                format!("{cwd}/{acc}")
            };
        }
        let n = normalize(&acc);
        if n.len() > 1 {
            n.trim_end_matches('/').to_string()
        } else {
            n
        }
    }

    /// `path.relative`.
    pub fn relative(cwd: &str, from: &str, to: &str) -> String {
        let (from, to) = (resolve(cwd, &[from]), resolve(cwd, &[to]));
        if from == to {
            return String::new();
        }
        let f: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
        let t: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
        let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
        let mut out: Vec<&str> = vec![".."; f.len() - common];
        out.extend(&t[common..]);
        out.join("/")
    }
}

// ---------------------------------------------------------------------------
// `@iarna/toml` 2.2.5 values: parse (on the `toml` crate) and stringify.

/// A parsed TOML value as the JS holds it (plus `undefined`/`null`, which
/// the resolvers can put into a manifest object).
#[derive(Clone, Debug, PartialEq)]
enum Tv {
    Undefined,
    Null,
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Arr(Vec<Tv>),
    Table(Table),
}

/// A JS object: insertion-ordered keys, integer-like keys enumerated first.
#[derive(Clone, Debug, Default, PartialEq)]
struct Table(Vec<(String, Tv)>);

impl Table {
    fn get(&self, key: &str) -> Option<&Tv> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut Tv> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Assignment: an existing key keeps its position.
    fn set(&mut self, key: &str, value: Tv) {
        match self.get_mut(key) {
            Some(slot) => *slot = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    /// `delete`.
    fn remove(&mut self, key: &str) {
        self.0.retain(|(k, _)| k != key);
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `Object.keys` order.
    fn order(&self) -> Vec<usize> {
        let index = |k: &str| -> Option<u32> {
            let canonical = k == "0"
                || (!k.is_empty() && !k.starts_with('0') && k.bytes().all(|b| b.is_ascii_digit()));
            if !canonical {
                return None;
            }
            k.parse::<u64>()
                .ok()
                .filter(|n| *n < u64::from(u32::MAX))
                .map(|n| n as u32)
        };
        let mut ints: Vec<(u32, usize)> = self
            .0
            .iter()
            .enumerate()
            .filter_map(|(i, (k, _))| index(k).map(|n| (n, i)))
            .collect();
        ints.sort_unstable();
        let mut out: Vec<usize> = ints.into_iter().map(|(_, i)| i).collect();
        out.extend((0..self.0.len()).filter(|i| index(&self.0[*i].0).is_none()));
        out
    }

    fn keys(&self) -> Vec<String> {
        self.order()
            .into_iter()
            .map(|i| self.0[i].0.clone())
            .collect()
    }

    fn entries(&self) -> Vec<(&str, &Tv)> {
        self.order()
            .into_iter()
            .map(|i| (self.0[i].0.as_str(), &self.0[i].1))
            .collect()
    }

    /// `a?.b?.c`.
    fn prop_path(&self, path: &[&str]) -> Option<&Tv> {
        let (first, rest) = path.split_first()?;
        rest.iter().try_fold(self.get(first)?, |v, k| v.prop(k))
    }

    fn prop_path_mut(&mut self, path: &[&str]) -> Option<&mut Tv> {
        let (first, rest) = path.split_first()?;
        let mut cur = self.get_mut(first)?;
        for key in rest {
            cur = match cur {
                Tv::Table(t) => t.get_mut(key)?,
                _ => return None,
            };
        }
        Some(cur)
    }
}

impl Tv {
    /// Property access: only tables have properties here.
    fn prop(&self, key: &str) -> Option<&Tv> {
        match self {
            Tv::Table(t) => t.get(key),
            _ => None,
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Tv::Undefined | Tv::Null => false,
            Tv::Str(s) => !s.is_empty(),
            Tv::Int(i) => *i != 0,
            Tv::Float(f) => *f != 0.0 && !f.is_nan(),
            Tv::Bool(b) => *b,
            Tv::Arr(_) | Tv::Table(_) => true,
        }
    }

    /// `===`: objects are never equal to a fresh value.
    fn strict_eq(&self, other: &Tv) -> bool {
        match (self, other) {
            (Tv::Arr(_) | Tv::Table(_), _) | (_, Tv::Arr(_) | Tv::Table(_)) => false,
            (Tv::Int(a), Tv::Float(b)) | (Tv::Float(b), Tv::Int(a)) => (*a as f64) == *b,
            (a, b) => a == b,
        }
    }

    /// JS `String(value)`.
    fn js_str(&self) -> String {
        match self {
            Tv::Undefined => "undefined".into(),
            Tv::Null => "null".into(),
            Tv::Str(s) => s.clone(),
            Tv::Int(i) => i.to_string(),
            Tv::Float(f) => jstoml::js_number(*f),
            Tv::Bool(b) => b.to_string(),
            Tv::Arr(items) => items
                .iter()
                .map(|v| {
                    if matches!(v, Tv::Undefined | Tv::Null) {
                        String::new()
                    } else {
                        v.js_str()
                    }
                })
                .collect::<Vec<_>>()
                .join(","),
            Tv::Table(_) => "[object Object]".into(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Tv::Undefined => "undefined",
            Tv::Null => "null",
            Tv::Str(_) => "string",
            Tv::Int(_) | Tv::Float(_) => "number",
            Tv::Bool(_) => "boolean",
            Tv::Arr(_) => "array",
            Tv::Table(_) => "table",
        }
    }
}

mod jstoml {
    use eyre::{Result, bail, eyre};
    use toml::de::{DeTable, DeValue};

    use super::{Table, Tv};

    /// `@iarna/toml` `parse`: key order = first appearance in the document,
    /// numbers unboxed to JS numbers, mixed-type arrays rejected (TOML 0.5).
    pub fn parse(src: &str) -> Result<Table> {
        let doc = DeTable::parse(src).map_err(|e| eyre!("{e}"))?;
        table(doc.get_ref())
    }

    fn table(t: &DeTable<'_>) -> Result<Table> {
        let mut entries = Vec::with_capacity(t.len());
        for (k, v) in t.iter() {
            entries.push((k.span().start, k.get_ref().to_string(), value(v.get_ref())?));
        }
        entries.sort_by_key(|(pos, _, _)| *pos);
        Ok(Table(entries.into_iter().map(|(_, k, v)| (k, v)).collect()))
    }

    fn value(v: &DeValue<'_>) -> Result<Tv> {
        Ok(match v {
            DeValue::String(s) => Tv::Str(s.to_string()),
            DeValue::Integer(i) => Tv::Int(
                i64::from_str_radix(i.as_str(), i.radix())
                    .map_err(|e| eyre!("integer {i}: {e}"))?,
            ),
            DeValue::Float(f) => {
                Tv::Float(f.as_str().parse().map_err(|e| eyre!("float {f}: {e}"))?)
            }
            DeValue::Boolean(b) => Tv::Bool(*b),
            DeValue::Datetime(d) => bail!(
                "TOML datetime {d}: @iarna/toml turns it into a JS Date whose rendering butler \
                 does not reproduce"
            ),
            DeValue::Array(items) => {
                let items = items
                    .iter()
                    .map(|i| value(i.get_ref()))
                    .collect::<Result<Vec<_>>>()?;
                if let Some(first) = items.first()
                    && let Some(other) = items.iter().find(|i| kind(i) != kind(first))
                {
                    bail!(
                        "Inline lists must be a single type, not a mix of {} and {}",
                        kind(first),
                        kind(other)
                    );
                }
                Tv::Arr(items)
            }
            DeValue::Table(t) => Tv::Table(table(t)?),
        })
    }

    /// The parser's element type (ints and floats differ here, unlike in
    /// `stringify`).
    fn kind(v: &Tv) -> &'static str {
        match v {
            Tv::Int(_) => "integer",
            Tv::Float(_) => "float",
            other => other.kind(),
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Ty {
        Undefined,
        Null,
        Integer,
        Float,
        Boolean,
        String,
        StringLiteral,
        StringMultiline,
        Array,
        Table,
    }

    /// `tomlType` of the stringifier: integral numbers are integers.
    fn toml_type(v: &Tv) -> Ty {
        match v {
            Tv::Undefined => Ty::Undefined,
            Tv::Null => Ty::Null,
            Tv::Int(_) => Ty::Integer,
            Tv::Float(f)
                if f.is_finite() && f.fract() == 0.0 && !(*f == 0.0 && f.is_sign_negative()) =>
            {
                Ty::Integer
            }
            Tv::Float(_) => Ty::Float,
            Tv::Bool(_) => Ty::Boolean,
            Tv::Str(_) => Ty::String,
            Tv::Arr(_) => Ty::Array,
            Tv::Table(_) => Ty::Table,
        }
    }

    fn is_inline(v: &Tv) -> bool {
        match v {
            Tv::Arr(items) => items.first().is_none_or(|f| toml_type(f) != Ty::Table),
            Tv::Table(t) => t.is_empty(),
            _ => true,
        }
    }

    /// `@iarna/toml` `stringify`.
    pub fn stringify(t: &Table) -> Result<String> {
        object("", "", t)
    }

    fn object(prefix: &str, indent: &str, t: &Table) -> Result<String> {
        let entries = t.entries();
        let inline: Vec<_> = entries.iter().filter(|(_, v)| is_inline(v)).collect();
        let complex: Vec<_> = entries.iter().filter(|(_, v)| !is_inline(v)).collect();
        let mut result = Vec::new();
        for (k, v) in &inline {
            if !matches!(toml_type(v), Ty::Undefined | Ty::Null) {
                result.push(format!("{indent}{} = {}", key(k), any_inline(v, true)?));
            }
        }
        if !result.is_empty() {
            result.push(String::new());
        }
        let complex_indent = if !prefix.is_empty() && !inline.is_empty() {
            format!("{indent}  ")
        } else {
            String::new()
        };
        for (k, v) in complex {
            result.push(match v {
                Tv::Arr(items) => array_of_tables(prefix, &complex_indent, k, items)?,
                Tv::Table(t) => complex_table(prefix, &complex_indent, k, t)?,
                other => bail!("Can only stringify objects, not {}", other.kind()),
            });
        }
        Ok(result.join("\n"))
    }

    fn key(k: &str) -> String {
        if !k.is_empty()
            && k.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            k.to_string()
        } else {
            basic_string(k)
        }
    }

    fn escape(s: &str) -> String {
        let s = s
            .replace('\\', "\\\\")
            .replace('\u{8}', "\\b")
            .replace('\t', "\\t")
            .replace('\n', "\\n")
            .replace('\u{c}', "\\f")
            .replace('\r', "\\r");
        // The control-character regex has no `g` flag: first match only.
        match s
            .char_indices()
            .find(|(_, c)| matches!(*c, '\u{0}'..='\u{1f}' | '\u{7f}'))
        {
            Some((i, c)) => format!("{}\\u{:04x}{}", &s[..i], c as u32, &s[i + c.len_utf8()..]),
            None => s,
        }
    }

    fn basic_string(s: &str) -> String {
        format!("\"{}\"", escape(s).replace('"', "\\\""))
    }

    fn multiline_string(s: &str) -> String {
        let mut escaped = s
            .split('\n')
            .map(|line| {
                let line: Vec<char> = escape(line).chars().collect();
                let mut out = String::with_capacity(line.len());
                for (i, c) in line.iter().enumerate() {
                    if *c == '"' && line.get(i + 1) == Some(&'"') && line.get(i + 2) == Some(&'"') {
                        out.push('\\');
                    }
                    out.push(*c);
                }
                out
            })
            .collect::<Vec<_>>()
            .join("\n");
        if escaped.ends_with('"') {
            escaped.push_str("\\\n");
        }
        format!("\"\"\"\n{escaped}\"\"\"")
    }

    fn any_inline(v: &Tv, multiline_ok: bool) -> Result<String> {
        let mut ty = toml_type(v);
        if let Tv::Str(s) = v {
            if multiline_ok && s.contains('\n') {
                ty = Ty::StringMultiline;
            } else if !s.contains(['\u{8}', '\t', '\n', '\u{c}', '\r', '\'']) && s.contains('"') {
                ty = Ty::StringLiteral;
            }
        }
        inline(v, ty)
    }

    fn inline(v: &Tv, ty: Ty) -> Result<String> {
        Ok(match (ty, v) {
            (Ty::StringMultiline, Tv::Str(s)) => multiline_string(s),
            (Ty::StringLiteral, Tv::Str(s)) => format!("'{s}'"),
            (Ty::String, Tv::Str(s)) => basic_string(s),
            (Ty::Integer, Tv::Int(i)) => thousands(&i.to_string()),
            (Ty::Integer, Tv::Float(f)) => thousands(&js_number(*f)),
            (Ty::Float, Tv::Int(i)) => format!("{}.0", thousands(&i.to_string())),
            (Ty::Float, Tv::Float(f)) => float(*f),
            (Ty::Boolean, Tv::Bool(b)) => b.to_string(),
            (Ty::Array, Tv::Arr(items)) => inline_array(items)?,
            (Ty::Table, Tv::Table(t)) => inline_table(t)?,
            (ty, v) => bail!("Can only stringify objects, not {} as {ty:?}", v.kind()),
        })
    }

    /// `stringifyInteger`: `_` before every run of three digits that ends
    /// the digit run, at non-word-boundaries.
    fn thousands(s: &str) -> String {
        let c: Vec<char> = s.chars().collect();
        let word = |x: Option<&char>| x.is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_');
        let mut out = String::with_capacity(s.len() + s.len() / 3);
        for (i, ch) in c.iter().enumerate() {
            let non_boundary = word(i.checked_sub(1).and_then(|p| c.get(p))) == word(c.get(i));
            let run = c[i..].iter().take_while(|d| d.is_ascii_digit()).count();
            if non_boundary && run > 0 && run % 3 == 0 {
                out.push('_');
            }
            out.push(*ch);
        }
        out
    }

    /// `stringifyFloat`.
    fn float(f: f64) -> String {
        if f == f64::INFINITY {
            return "inf".into();
        }
        if f == f64::NEG_INFINITY {
            return "-inf".into();
        }
        if f.is_nan() {
            return "nan".into();
        }
        if f == 0.0 && f.is_sign_negative() {
            return "-0.0".into();
        }
        let s = js_number(f);
        let mut chunks = s.split('.');
        let int = chunks.next().unwrap_or_default();
        let dec = chunks.next().unwrap_or("0");
        format!("{}.{dec}", thousands(int))
    }

    /// ECMAScript `Number::toString(10)`.
    pub fn js_number(f: f64) -> String {
        if f.is_nan() {
            return "NaN".into();
        }
        if f == 0.0 {
            return "0".into();
        }
        if f.is_infinite() {
            return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
        }
        let sign = if f < 0.0 { "-" } else { "" };
        let sci = format!("{:e}", f.abs());
        let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
        let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
        let k = digits.len() as i64;
        let n = exp.parse::<i64>().unwrap_or(0) + 1;
        let body = if k <= n && n <= 21 {
            format!("{digits}{}", "0".repeat((n - k) as usize))
        } else if 0 < n && n <= 21 {
            format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
        } else if -6 < n && n <= 0 {
            format!("0.{}{digits}", "0".repeat((-n) as usize))
        } else {
            let e = n - 1;
            let e = if e >= 0 {
                format!("+{e}")
            } else {
                e.to_string()
            };
            if k == 1 {
                format!("{digits}e{e}")
            } else {
                format!("{}.{}e{e}", &digits[..1], &digits[1..])
            }
        };
        format!("{sign}{body}")
    }

    fn array_type(items: &[Tv]) -> Result<Ty> {
        let Some(first) = items.first() else {
            return Ok(Ty::Undefined);
        };
        let ty = toml_type(first);
        if items.iter().all(|i| toml_type(i) == ty) {
            return Ok(ty);
        }
        if items
            .iter()
            .all(|i| matches!(toml_type(i), Ty::Integer | Ty::Float))
        {
            return Ok(Ty::Float);
        }
        bail!("Array values can't have mixed types")
    }

    fn inline_array(items: &[Tv]) -> Result<String> {
        let items: Vec<&Tv> = items
            .iter()
            .filter(|i| !matches!(i, Tv::Undefined | Tv::Null))
            .collect();
        let owned: Vec<Tv> = items.iter().map(|i| (*i).clone()).collect();
        let ty = array_type(&owned)?;
        let parts = owned
            .iter()
            .map(|i| inline(i, ty))
            .collect::<Result<Vec<_>>>()?;
        let joined = parts.join(", ");
        Ok(
            if joined.encode_utf16().count() > 60 || parts.iter().any(|p| p.contains('\n')) {
                format!("[\n  {}\n]", parts.join(",\n  "))
            } else {
                format!("[ {joined}{}]", if parts.is_empty() { "" } else { " " })
            },
        )
    }

    fn inline_table(t: &Table) -> Result<String> {
        let parts = t
            .entries()
            .into_iter()
            .map(|(k, v)| Ok(format!("{} = {}", key(k), any_inline(v, false)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(format!(
            "{{ {}{}}}",
            parts.join(", "),
            if parts.is_empty() { "" } else { " " }
        ))
    }

    fn array_of_tables(prefix: &str, indent: &str, k: &str, items: &[Tv]) -> Result<String> {
        array_type(items)?;
        let full = format!("{prefix}{}", key(k));
        let mut out = String::new();
        for item in items {
            let Tv::Table(t) = item else {
                bail!("Can only stringify objects, not {}", item.kind())
            };
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("{indent}[[{full}]]\n"));
            out.push_str(&object(&format!("{full}."), indent, t)?);
        }
        Ok(out)
    }

    fn complex_table(prefix: &str, indent: &str, k: &str, t: &Table) -> Result<String> {
        let full = format!("{prefix}{}", key(k));
        let mut out = String::new();
        if t.0.iter().any(|(_, v)| is_inline(v)) {
            out.push_str(&format!("{indent}[{full}]\n"));
        }
        out.push_str(&object(&format!("{full}."), indent, t)?);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::args;
    use crate::graph::{Project, ProjectGraph};

    /// A throwaway workspace directory.
    struct Ws(PathBuf);

    impl Ws {
        fn new(files: &[(&str, &str)]) -> Ws {
            let root =
                std::env::temp_dir().join(format!("butler-nxlv-{}", uuid_v4().expect("uuid")));
            for (path, content) in files {
                let p = root.join(path);
                fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
                fs::write(&p, content).expect("write");
            }
            Ws(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.0.join(rel)).expect("read")
        }
    }

    impl Drop for Ws {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn project(name: &str, root: &str, build_options: Option<Json>) -> Project {
        let mut targets = BTreeMap::new();
        if let Some(options) = build_options {
            let target = json!({"executor": "@nxlv/python:build", "options": options});
            targets.insert(
                "build".to_string(),
                serde_json::from_value(target).expect("target"),
            );
        }
        Project {
            name: name.into(),
            root: root.into(),
            project_type: None,
            tags: vec![],
            implicit_dependencies: vec![],
            targets,
            deps: Default::default(),
            build_deps: Default::default(),
        }
    }

    fn graph(projects: Vec<Project>) -> ProjectGraph {
        ProjectGraph {
            projects: projects.into_iter().map(|p| (p.name.clone(), p)).collect(),
            ..Default::default()
        }
    }

    fn base_env() -> Env {
        Env::from([("PATH".to_string(), "/usr/bin:/bin".to_string())])
    }

    struct Call<'a> {
        ws: &'a Path,
        graph: &'a ProjectGraph,
        project: &'a str,
        env: &'a Env,
    }

    impl Call<'_> {
        fn plan(&self, executor: &str, options: Json, cli: &[&str]) -> Result<Plan> {
            let unparsed: Vec<String> = cli.iter().map(|s| (*s).to_string()).collect();
            let mut overrides = args::parse(&unparsed, args::OVERRIDES);
            if overrides
                .get("_")
                .and_then(Json::as_array)
                .is_some_and(Vec::is_empty)
            {
                overrides.remove("_");
            }
            let options: JsonMap = serde_json::from_value(options).expect("options");
            let ctx = PlanCtx {
                workspace_root: self.ws,
                graph: self.graph,
                project: &self.graph.projects[self.project],
                target: "t",
                configuration: None,
                options: &options,
                overrides: &overrides,
                unparsed: &unparsed,
                env: self.env,
            };
            plan(executor, &ctx)
        }

        fn rendered(&self, executor: &str, options: Json, cli: &[&str]) -> Vec<String> {
            let plan = self.plan(executor, options, cli).expect("plan");
            assert!(!plan.parallel);
            plan.steps.iter().map(ToString::to_string).collect()
        }
    }

    fn run_native(step: &Step, ws: &Path, env: &Env) -> StepOutput {
        let Step::Native { run, .. } = step else {
            panic!("not a native step: {step}")
        };
        run(&NativeCtx {
            workspace_root: ws,
            env,
        })
        .expect("native step")
    }

    const CHECK: &str = "check that `uv` is installed (command-exists uv)";

    const FAKE_UV: &str = r#"#!/bin/sh
[ -n "$FAKE_UV_LOG" ] && echo "$*" >> "$FAKE_UV_LOG"
case "$1" in
  --version) echo "uv 0.12.18 (fake)";;
  export) printf -- '-e ./libs/lib\nrequests==2.32.3\nurllib3==2.2.3\n';;
  build) mkdir -p dist && cp pyproject.toml dist/pyproject.toml && find . -path ./dist -prune -o -type f -print | sort > dist/files.txt;;
  *) echo "unexpected: $*" >&2; exit 3;;
esac
"#;

    /// A uv workspace (root `uv.lock`) with an app bundling a local lib.
    fn uv_workspace() -> Ws {
        let ws = Ws::new(&[
            ("bin/uv", FAKE_UV),
            (
                "pyproject.toml",
                "[project]\nname = \"ws\"\nversion = \"0.0.0\"\ndependencies = [\"app\"]\n\n\
                 [tool.uv.workspace]\nmembers = [\"apps/app\", \"libs/lib\"]\n",
            ),
            (
                "apps/app/pyproject.toml",
                "# app manifest\n[project]\nname = \"app\"\nversion = \"1.0.0\"\n\
                 dependencies = [\"lib\", \"requests>=2\"]\n\n\
                 [project.optional-dependencies]\nsocks = [\"requests[socks]\"]\n\n\
                 [dependency-groups]\ndev = [\"pytest>=8\"]\n\n\
                 [tool.uv.sources]\nlib = { workspace = true }\n\n\
                 [tool.uv.build-backend]\nmodule-name = \"app\"\n\n\
                 [build-system]\nrequires = [\"uv_build>=0.8.9,<0.9.0\"]\nbuild-backend = \"uv_build\"\n",
            ),
            ("apps/app/src/app/__init__.py", "print(\"app\")\n"),
            ("apps/app/src/app/__pycache__/x.pyc", "junk\n"),
            ("apps/app/tests/test_x.py", "def test(): pass\n"),
            ("apps/app/README.md", "readme\n"),
            (
                "libs/lib/pyproject.toml",
                "[project]\nname = \"lib\"\nversion = \"0.1.0\"\ndependencies = []\n\n\
                 [build-system]\nrequires = [\"uv_build>=0.8.9,<0.9.0\"]\nbuild-backend = \"uv_build\"\n",
            ),
            ("libs/lib/src/lib/__init__.py", "print(\"lib\")\n"),
            (
                "uv.lock",
                "version = 1\nrequires-python = \">=3.10\"\n\n\
                 [[package]]\nname = \"app\"\nversion = \"1.0.0\"\nsource = { editable = \"apps/app\" }\n\
                 dependencies = [{ name = \"lib\" }, { name = \"requests\" }]\n\n\
                 [package.optional-dependencies]\nsocks = [{ name = \"requests\", extra = [\"socks\"] }]\n\n\
                 [[package]]\nname = \"lib\"\nversion = \"0.1.0\"\nsource = { editable = \"libs/lib\" }\n\n\
                 [[package]]\nname = \"pysocks\"\nversion = \"1.7.1\"\nsource = { registry = \"https://pypi.org/simple\" }\n\n\
                 [[package]]\nname = \"requests\"\nversion = \"2.32.3\"\nsource = { registry = \"https://pypi.org/simple\" }\n\
                 dependencies = [{ name = \"urllib3\" }]\n\n\
                 [package.optional-dependencies]\nsocks = [{ name = \"pysocks\" }]\n\n\
                 [[package]]\nname = \"urllib3\"\nversion = \"2.2.3\"\nsource = { registry = \"https://pypi.org/simple\" }\n",
            ),
        ]);
        fs::set_permissions(ws.path().join("bin/uv"), fs::Permissions::from_mode(0o755))
            .expect("chmod");
        ws
    }

    /// Per-project environments (no root `uv.lock`): app -> lib -> deep via
    /// path sources, plus a published `pub`.
    fn per_project() -> Ws {
        let hatch =
            "[build-system]\nrequires = [\"hatchling\"]\nbuild-backend = \"hatchling.build\"\n";
        Ws::new(&[
            (
                "apps/app/pyproject.toml",
                &format!(
                    "[project]\nname = \"app\"\nversion = \"1.0.0\"\n\
                     dependencies = [\"lib[color]\", \"click>=8\", \"pub\"]\n\n\
                     [tool.uv.sources]\nlib = {{ path = \"../../libs/lib\" }}\npub = {{ path = \"../../libs/pub\" }}\n\n\
                     [[tool.uv.index]]\nname = \"private\"\nurl = \"https://other.example/simple\"\n\n\
                     [tool.hatch.build.targets.wheel]\npackages = [\"app\"]\n\n{hatch}"
                ),
            ),
            ("apps/app/app/__init__.py", "x = 1\n"),
            (
                "libs/lib/pyproject.toml",
                &format!(
                    "[project]\nname = \"lib\"\nversion = \"0.1.0\"\ndependencies = [\"rich\", \"deep\"]\n\n\
                     [project.optional-dependencies]\ncolor = [\"colored>=2.3.0\"]\n\
                     json = [\"python-json-logger>=2.0.4\"]\n\n\
                     [tool.uv.sources]\ndeep = {{ path = \"../deep\" }}\n\n{hatch}"
                ),
            ),
            ("libs/lib/src/lib/__init__.py", "y = 1\n"),
            (
                "libs/deep/pyproject.toml",
                &format!(
                    "[project]\nname = \"deep\"\nversion = \"0.2.0\"\ndependencies = [\"six\"]\n\n\
                     [tool.hatch.build.targets.wheel]\npackages = [\"deep\"]\n\n{hatch}"
                ),
            ),
            ("libs/deep/deep/__init__.py", "z = 1\n"),
            (
                "libs/pub/pyproject.toml",
                &format!(
                    "[project]\nname = \"pub\"\nversion = \"3.1.0\"\ndependencies = []\n\n{hatch}"
                ),
            ),
            ("libs/pub/pub/__init__.py", "p = 1\n"),
        ])
    }

    fn per_project_graph() -> ProjectGraph {
        graph(vec![
            project("app", "apps/app", None),
            project("deep", "libs/deep", None),
            project("lib", "libs/lib", Some(json!({"publish": false}))),
            project(
                "pub",
                "libs/pub",
                Some(
                    json!({"customSourceUrl": "https://pkgs.example/simple", "customSourceName": "private"}),
                ),
            ),
        ])
    }

    #[test]
    fn ruff_format_raw_check_flag_goes_last() {
        let ws = uv_workspace();
        let g = graph(vec![project("app", "apps/app", None)]);
        let env = base_env();
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &env,
        };
        let opts = json!({"filePatterns": ["app", "tests"]});
        assert_eq!(
            call.rendered("ruff-format", opts.clone(), &["--check"]),
            vec![
                CHECK,
                "(cd apps/app && uv run ruff format app tests --check)"
            ]
        );
        // `--check false` is consumed and wins over the option.
        let opts = json!({"filePatterns": ["app"], "check": true});
        assert_eq!(
            call.rendered("ruff-format", opts, &["--check", "false", "--diff"]),
            vec![CHECK, "(cd apps/app && uv run ruff format app --diff)"]
        );
    }

    #[test]
    fn ruff_check_forwards_unparsed_before_flags() {
        let ws = uv_workspace();
        let g = graph(vec![project("app", "apps/app", None)]);
        let env = base_env();
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &env,
        };
        let opts = json!({"lintFilePatterns": ["app", "tests"], "exitZero": true});
        assert_eq!(
            call.rendered("ruff-check", opts, &["--select", "E501", "--fix", "true"]),
            vec![
                CHECK,
                "(cd apps/app && uv run ruff check app tests --select E501 --fix --exit-zero)"
            ]
        );
        let missing = call
            .plan("ruff-check", json!({}), &[])
            .err()
            .expect("lintFilePatterns is required");
        assert!(
            missing.to_string().contains("lintFilePatterns"),
            "{missing}"
        );
    }

    #[test]
    fn workspace_mode_runs_uv_at_the_root_with_project() {
        let ws = uv_workspace();
        let g = graph(vec![
            project("app", "apps/app", None),
            project("lib", "libs/lib", None),
        ]);
        let env = base_env();
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &env,
        };
        let r = |executor: &str, opts: Json, cli: &[&str]| {
            call.rendered(executor, opts, cli)[1..].to_vec()
        };
        assert_eq!(r("lock", json!({"update": false}), &[]), vec!["uv lock"]);
        // lock keeps empty pieces of `args`; verbosity after args.
        assert_eq!(
            r(
                "lock",
                json!({"update": true, "args": "--offline  -q", "debug": true}),
                &[]
            ),
            vec!["uv lock --upgrade --offline '' -q -vvv"]
        );
        // install/sync: verbosity before args, empty pieces dropped.
        assert_eq!(
            r(
                "sync",
                json!({"args": "--frozen  --all-extras", "verbose": true, "cacheDir": "/c"}),
                &[]
            ),
            vec!["uv sync -v --frozen --all-extras --cache-dir /c"]
        );
        assert_eq!(
            r(
                "install",
                json!({"silent": false, "args": "", "verbose": false, "debug": false}),
                &[]
            ),
            vec!["uv sync"]
        );
        assert_eq!(
            r(
                "add",
                json!({}),
                &[
                    "--name",
                    "requests",
                    "--group",
                    "dev",
                    "--extras",
                    "socks",
                    "--args=--no-sync"
                ]
            ),
            vec!["uv add requests --group dev --extra socks --no-sync --project apps/app"]
        );
        // `local` only matters outside a workspace.
        assert_eq!(
            r("add", json!({"local": true}), &["--name", "lib"]),
            vec!["uv add lib --project apps/app"]
        );
        assert_eq!(
            r("update", json!({}), &["--name", "requests"]),
            vec![
                "uv lock --upgrade-package requests --project apps/app",
                "uv sync"
            ]
        );
        assert_eq!(
            r(
                "remove",
                json!({"args": "--frozen"}),
                &["--name", "requests"]
            ),
            vec!["uv remove requests --project apps/app --frozen"]
        );
        assert!(
            call.plan("update", json!({}), &[]).is_err(),
            "update needs a package name"
        );
    }

    #[test]
    fn per_project_mode_syncs_dependents_transitively() {
        let ws = per_project();
        let g = per_project_graph();
        let env = base_env();
        let deep = Call {
            ws: ws.path(),
            graph: &g,
            project: "deep",
            env: &env,
        };
        // lib's sources name deep, app's name lib: both re-sync, in order.
        assert_eq!(
            deep.rendered("add", json!({"name": "requests"}), &[]),
            vec![
                CHECK,
                "(cd libs/deep && uv add requests)",
                "(cd libs/lib && uv sync)",
                "(cd apps/app && uv sync)"
            ]
        );
        let app = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &env,
        };
        assert_eq!(
            app.rendered("add", json!({"name": "pub", "local": true}), &[]),
            vec![CHECK, "(cd apps/app && uv add --editable ../../libs/pub)"]
        );
        assert_eq!(
            app.rendered("lock", json!({}), &[]),
            vec![CHECK, "(cd apps/app && uv lock)"]
        );
        let unknown = app
            .plan("add", json!({"name": "nope", "local": true}), &[])
            .err()
            .expect("unknown local project");
        assert!(
            unknown
                .to_string()
                .contains("project nope not found in the Nx workspace"),
            "{unknown}"
        );
    }

    #[test]
    fn provider_selection_refuses_poetry() {
        let ws = Ws::new(&[
            ("apps/p/pyproject.toml", "[tool.poetry]\nname = \"p\"\n"),
            ("apps/q/README.md", "no manifest\n"),
            ("uv.lock", "version = 1\n"),
            ("poetry.lock", "\n"),
        ]);
        let g = graph(vec![
            project("p", "apps/p", None),
            project("q", "apps/q", None),
        ]);
        let env = base_env();
        let p = Call {
            ws: ws.path(),
            graph: &g,
            project: "p",
            env: &env,
        };
        let err = p
            .plan("lock", json!({}), &[])
            .err()
            .expect("poetry project");
        assert!(err.to_string().contains("poetry provider"), "{err}");
        // Without a project manifest the root lock files decide.
        let q = Call {
            ws: ws.path(),
            graph: &g,
            project: "q",
            env: &env,
        };
        let err = q
            .plan("lock", json!({}), &[])
            .err()
            .expect("both lock files");
        assert!(
            err.to_string().contains("Both poetry.lock and uv.lock"),
            "{err}"
        );
        let other = p
            .plan("tox", json!({}), &[])
            .err()
            .expect("unported executor");
        assert!(
            other.to_string().contains("@nxlv/python:tox is not ported"),
            "{other}"
        );
    }

    #[test]
    fn venv_activation_rules() {
        let ws = uv_workspace();
        let root = ws.path().to_str().expect("utf-8").to_string();
        let g = graph(vec![project("app", "apps/app", None)]);
        let opts = json!({"filePatterns": ["app"]});
        let shell_env = |env: &Env, opts: Json| -> Env {
            let call = Call {
                ws: ws.path(),
                graph: &g,
                project: "app",
                env,
            };
            let plan = call.plan("ruff-format", opts, &[]).expect("plan");
            match plan.steps.last() {
                Some(Step::Shell { env, .. }) => env.clone(),
                _ => panic!("last step is the shell"),
            }
        };
        // No [tool.nx] autoActivate: nothing changes.
        assert!(shell_env(&base_env(), opts.clone()).is_empty());

        fs::write(
            ws.path().join("pyproject.toml"),
            "[project]\nname = \"ws\"\n\n[tool.nx]\nautoActivate = true\n",
        )
        .expect("write");
        let venv = format!("{root}/.venv");
        assert_eq!(
            shell_env(&base_env(), opts.clone()),
            Env::from([
                ("PATH".to_string(), format!("{venv}/bin:/usr/bin:/bin")),
                ("VIRTUAL_ENV".to_string(), venv.clone())
            ])
        );
        // An active virtualenv is left alone.
        let mut active = base_env();
        active.insert("VIRTUAL_ENV".into(), "/elsewhere".into());
        assert!(shell_env(&active, opts.clone()).is_empty());
        // PYTHONHOME would have to be unset.
        let mut home = base_env();
        home.insert("PYTHONHOME".into(), "/py".into());
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &home,
        };
        let err = call
            .plan("ruff-format", opts.clone(), &[])
            .err()
            .expect("PYTHONHOME");
        assert!(err.to_string().contains("PYTHONHOME"), "{err}");

        // installDependenciesIfNotExists: a conditional `uv sync` first; the
        // venv is activated twice (autoActivate, then install), as in the JS.
        let install = json!({"filePatterns": ["app"], "installDependenciesIfNotExists": true});
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &base_env(),
        };
        let plan = call
            .plan("ruff-format", install.clone(), &[])
            .expect("plan");
        assert!(plan.steps[0].to_string().starts_with(
            "if ./.venv does not exist: check that `uv` is installed, then `uv sync` in ."
        ));
        assert_eq!(
            shell_env(&base_env(), install)["PATH"],
            format!("{venv}/bin:{venv}/bin:/usr/bin:/bin")
        );

        fs::write(
            ws.path().join("pyproject.toml"),
            "[project]\nname = \"ws\"\n",
        )
        .expect("write");
        let err = call
            .plan("ruff-format", opts, &[])
            .err()
            .expect("no [tool]");
        assert!(err.to_string().contains("no [tool] table"), "{err}");
    }

    #[test]
    fn run_commands_strips_install_option_and_layers_the_venv() {
        let ws = uv_workspace();
        let root = ws.path().to_str().expect("utf-8").to_string();
        let g = graph(vec![project("app", "apps/app", None)]);
        let env = base_env();
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &env,
        };
        let opts = json!({"command": "uv run pytest tests/", "cwd": "apps/app"});
        let plan = call.plan("run-commands", opts.clone(), &[]).expect("plan");
        let [Step::Shell { script, cwd, .. }] = &plan.steps[..] else {
            panic!("one shell step")
        };
        assert_eq!(
            (script.as_str(), cwd.as_str()),
            ("uv run pytest tests/", "apps/app")
        );

        let mut opts = opts;
        opts["installDependenciesIfNotExists"] = json!(true);
        let plan = call.plan("run-commands", opts, &[]).expect("plan");
        assert!(!plan.parallel);
        let [install, Step::Shell { script, env, .. }] = &plan.steps[..] else {
            panic!("install + shell")
        };
        assert!(install.to_string().contains("`uv sync` in ."), "{install}");
        // Not forwarded as an unknown option.
        assert_eq!(script, "uv run pytest tests/");
        assert_eq!(env["VIRTUAL_ENV"], format!("{root}/.venv"));
        assert!(
            env["PATH"].contains(&format!("{root}/.venv/bin:/usr/bin:/bin")),
            "{}",
            env["PATH"]
        );
    }

    #[test]
    fn locked_build_matches_the_js_output() {
        let ws = uv_workspace();
        let root = ws.path().to_str().expect("utf-8").to_string();
        let g = graph(vec![
            project("app", "apps/app", None),
            project("lib", "libs/lib", None),
        ]);
        fs::create_dir_all(ws.path().join("tmp")).expect("mkdir");
        let env = Env::from([
            ("PATH".to_string(), format!("{root}/bin:/usr/bin:/bin")),
            ("TMPDIR".to_string(), format!("{root}/tmp/")),
            ("FAKE_UV_LOG".to_string(), format!("{root}/uv.log")),
        ]);
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &env,
        };
        let opts = json!({"outputPath": "apps/app/dist", "lockedVersions": true, "bundleLocalDependencies": true});
        let plan = call.plan("build", opts, &[]).expect("plan");
        assert_eq!(plan.steps[0].to_string(), CHECK);
        assert!(run_native(&plan.steps[0], ws.path(), &env).success);
        let out = run_native(&plan.steps[1], ws.path(), &env);
        assert!(out.success, "{}", out.output);
        // Oracle: @nxlv/python 23.1.1 `UVProvider.build` on the same fixture.
        assert_eq!(
            ws.read("apps/app/dist/pyproject.toml"),
            "dependency-groups = { }\n\n[project]\nname = \"app\"\nversion = \"1.0.0\"\n\
             dependencies = [ \"requests==2.32.3\", \"urllib3==2.2.3\" ]\n\n\
             \x20 [project.optional-dependencies]\n  socks = [ \"pysocks==1.7.1\" ]\n\n\
             [tool.uv]\nsources = { }\n\n\
             \x20 [tool.uv.build-backend]\n  module-name = [ \"app\", \"lib\" ]\n\n\
             [build-system]\nrequires = [ \"uv_build>=0.8.9,<0.9.0\" ]\nbuild-backend = \"uv_build\"\n"
        );
        assert_eq!(
            ws.read("apps/app/dist/files.txt"),
            "./README.md\n./pyproject.toml\n./src/app/__init__.py\n./src/lib/__init__.py\n"
        );
        let log = ws.read("uv.log");
        let calls: Vec<&str> = log.lines().collect();
        assert_eq!(
            calls,
            vec![
                "--version",
                "export --format requirements-txt --no-hashes --no-header --no-annotate --frozen --no-emit-project --project apps/app --no-dev",
                "build"
            ]
        );
        // The temp build folder is gone.
        let left = fs::read_dir(ws.path().join("tmp/nx-python/build"))
            .expect("build root")
            .count();
        assert_eq!(left, 0);
    }

    #[test]
    fn project_resolver_build_matches_the_js_output() {
        let ws = per_project();
        let root = ws.path().to_str().expect("utf-8").to_string();
        let g = per_project_graph();
        let env = base_env();
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &env,
        };
        let opts = json!({
            "outputPath": "apps/app/dist",
            "lockedVersions": false,
            "bundleLocalDependencies": false,
            "skipBuild": true,
            "keepBuildFolder": true,
            "buildFolder": format!("{root}/rsbuild"),
        });
        let plan = call.plan("build", opts, &[]).expect("plan");
        let out = run_native(&plan.steps[1], ws.path(), &env);
        assert!(out.success, "{}", out.output);
        assert_eq!(out.output.matches("Duplicate index for private renamed to private-f4e066b5ec9d7247a61b12593262cd8e").count(), 2);
        // Oracle: @nxlv/python 23.1.1 `ProjectDependencyResolver` on the same
        // fixture (lib bundled because publish=false, pub pinned to its own
        // index, deep pinned one level down).
        assert_eq!(
            ws.read("rsbuild/pyproject.toml"),
            "dependency-groups = { }\n\n[project]\nname = \"app\"\nversion = \"1.0.0\"\n\
             dependencies = [\n  \"click>=8\",\n  \"pub==3.1.0\",\n  \"rich\",\n  \"deep==0.2.0\",\n  \"colored>=2.3.0\"\n]\n\n\
             \x20 [project.optional-dependencies]\n  json = [ \"python-json-logger>=2.0.4\" ]\n\n\
             [tool.uv.sources.pub]\nindex = \"private-f4e066b5ec9d7247a61b12593262cd8e\"\n\n\
             [[tool.uv.index]]\nname = \"private\"\nurl = \"https://other.example/simple\"\n\n\
             [[tool.uv.index]]\nname = \"private-f4e066b5ec9d7247a61b12593262cd8e\"\nurl = \"https://pkgs.example/simple\"\n\n\
             [tool.hatch.build.targets.wheel]\npackages = [ \"app\", \"lib\" ]\n\n\
             [build-system]\nrequires = [ \"hatchling\" ]\nbuild-backend = \"hatchling.build\"\n"
        );
        assert!(ws.path().join("rsbuild/lib/__init__.py").exists());
        assert!(
            !ws.path().join("apps/app/dist").exists(),
            "skipBuild leaves outputPath alone"
        );
    }

    #[test]
    fn locked_without_bundling_is_refused() {
        let ws = uv_workspace();
        let g = graph(vec![project("app", "apps/app", None)]);
        let env = base_env();
        let call = Call {
            ws: ws.path(),
            graph: &g,
            project: "app",
            env: &env,
        };
        let opts =
            json!({"outputPath": "d", "lockedVersions": true, "bundleLocalDependencies": false});
        let err = call.plan("build", opts, &[]).err().expect("refused");
        assert!(
            err.to_string()
                .contains("cannot use lockedVersions without bundleLocalDependencies"),
            "{err}"
        );
    }

    #[test]
    fn stringify_round_trips_like_iarna_toml() {
        // Oracle: `@iarna/toml` 2.2.5 parse + stringify of the same input.
        let src = "\ntitle = \"x\"\nnum = 1000\nflt = 1.5\nbig = 1e3\nneg = -0.0\ntiny = 1.5e-7\nhuge = 1e22\n\
                   s1 = 'say \"hi\"'\ns2 = \"multi\\nline\"\ns3 = \"ctl\\u0001a\\u0002b\"\nempty = []\n\
                   long = [\"aaaaaaaaaaaaaaa\", \"bbbbbbbbbbbbbbbbbbb\", \"ccccccccccccccccc\", \"ddddddddd\"]\n\
                   nested = [[1, 2], [\"a\"]]\n\"key with space\" = true\n2 = \"two\"\n1 = \"one\"\n\
                   [tbl]\ninl = { a = 1, \"b c\" = 'q\"q' }\n[tbl.sub]\nx = 1\n\
                   [[aot]]\nn = 1\n[[aot]]\nn = 2\n[[aot.inner]]\nz = \"z\"\n[only.deep]\nk = \"v\"\n";
        let expected = "1 = \"one\"\n2 = \"two\"\ntitle = \"x\"\nnum = 1_000\nflt = 1.5\nbig = 1_000\nneg = -0.0\n\
                        tiny = 1.5e-7\nhuge = 1e+22\ns1 = 'say \"hi\"'\ns2 = \"\"\"\nmulti\nline\"\"\"\n\
                        s3 = \"ctl\\u0001a\u{2}b\"\nempty = [ ]\nlong = [\n  \"aaaaaaaaaaaaaaa\",\n  \"bbbbbbbbbbbbbbbbbbb\",\n  \
                        \"ccccccccccccccccc\",\n  \"ddddddddd\"\n]\nnested = [ [ 1, 2 ], [ \"a\" ] ]\n\
                        \"key with space\" = true\n\n[tbl.inl]\na = 1\n\"b c\" = 'q\"q'\n\n[tbl.sub]\nx = 1\n\n\
                        [[aot]]\nn = 1\n\n[[aot]]\nn = 2\n\n  [[aot.inner]]\n  z = \"z\"\n\n[only.deep]\nk = \"v\"\n";
        let parsed = jstoml::parse(src).expect("parse");
        assert_eq!(jstoml::stringify(&parsed).expect("stringify"), expected);
        // TOML 0.5 arrays are single-typed.
        let err = jstoml::parse("arr = [1, 2.5]\n").expect_err("mixed array");
        assert!(
            err.to_string().contains("mix of integer and float"),
            "{err}"
        );
        assert!(
            jstoml::parse("d = 1979-05-27\n").is_err(),
            "datetimes are refused"
        );
    }

    #[test]
    fn dependency_string_rules() {
        assert_eq!(
            normalize_dependency_name("requests[security]>=2.25"),
            Some("requests".into())
        );
        assert_eq!(
            normalize_dependency_name("zope.interface"),
            Some("zope".into())
        );
        assert_eq!(normalize_dependency_name(">=1"), None);
        assert_eq!(extract_extras(Some("lib[a, b]>=1")), vec!["a", "b"]);
        assert_eq!(
            extract_extras(Some("x[a]; extra == 'y[z]'")),
            vec!["a]; extra == 'y[z"]
        );
        assert!(extract_extras(Some("plain")).is_empty());
        for yes in [
            "lib==1.0",
            "lib >= 1.0, < 2",
            "lib~=1.2,!=1.3",
            "lib===1",
            "a.b-c_d<2",
        ] {
            assert!(version_range(yes), "{yes}");
        }
        for no in [
            "lib",
            "lib[x]==1",
            "lib==",
            "lib==1 ; python_version<'3'",
            "lib 1.0",
        ] {
            assert!(!version_range(no), "{no}");
        }
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            md5_hex(b"https://pkgs.example/simple"),
            "f4e066b5ec9d7247a61b12593262cd8e"
        );
    }

    #[test]
    fn ignore_paths_follow_minimatch() {
        let m = |p: &str, name: &str| IgnorePattern::new(p).expect("pattern").matches(name);
        assert!(m(".venv", ".venv"));
        assert!(m("*", ".venv"), "dot: true");
        assert!(m("*.{py,txt}", "a.py"));
        assert!(!m("tests/**", "tests"));
        assert!(m("**/tests", "tests"));
        assert!(m("!tests", "src") && !m("!tests", "tests"));
        assert!(!m("#tests", "tests"));
        assert!(IgnorePattern::new("a{1..3}").is_err());
        assert!(IgnorePattern::new("+(a|b)").is_err());
    }
}
