//! Port of `@nx-tools/nx-container:build` 7.3.0 with its docker engine
//! (`executor.js`, `context.js`, `engines/docker/*.js`), the `@nx-tools/core`
//! 7.3.0 input and interpolation helpers, and `@nx-tools/container-metadata`
//! + `@nx-tools/ci-context` 7.3.0 for the `metadata` option.
//!
//! Everything the executor derives from options and the environment is
//! resolved at plan time, so the step label is the `docker buildx build`
//! command itself: `INPUT_*` overrides, list parsing, argument order, `$VAR`
//! interpolation and — with `metadata.images` — the CI context plus the tags
//! and labels container-metadata computes (git is read, never written).
//! What depends on the machine runs inside one [`Step::Native`], in the
//! executor's order: probe docker/buildx, create the builder, look the
//! repository up on GitHub, write secrets into the temp dir, build, remove
//! the builder, write the `setOutput` files, remove the temp dir.
//!
//! The label renders run-time values as placeholders: `<tmp>` (the temp dir),
//! `<now>` (the image creation time), `<github:…>` (repository fields from the
//! GitHub API) and `<random>` (a created builder's suffix). It shows the
//! command a current buildx gets; the version-gated flags (`--build-context`
//! needs buildx >= 0.8.0, `--metadata-file` >= 0.6.0, `--iidfile` with
//! platforms >= 0.4.2) and `--secret` for a secret file that turns out to be
//! missing are dropped at run time exactly as the executor drops them.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use eyre::{Result, bail, eyre};
use serde_json::json;

use super::args::js_string;
use super::schema::combine_options;
use super::{NativeCtx, Plan, PlanCtx, Step, StepOutput, shell_join};
use crate::config::{Json, JsonMap};

type Env = BTreeMap<String, String>;

/// The relevant part of the executor's `schema.json`, verbatim.
fn schema() -> Json {
    let strings = json!({"type": "array", "items": {"type": "string"}});
    let string_or_strings = json!({"anyOf": [{"type": "string"}, strings]});
    let metadata = json!({
        "type": "object",
        "properties": {
            "images": strings,
            "tags": strings,
            "flavor": strings,
            "labels": strings,
            "sep-tags": {"type": "string", "default": "\n"},
            "sep-labels": {"type": "string", "default": "\n"},
            "bake-target": {"type": "string", "default": "container-metadata-action"}
        }
    });
    let string = json!({"type": "string"});
    let boolean = json!({"type": "boolean", "default": false});
    let properties = [
        ("engine", json!({"type": "string", "default": "docker"})),
        ("quiet", boolean.clone()),
        ("add-hosts", strings.clone()),
        ("allow", strings.clone()),
        ("build-args", strings.clone()),
        ("build-contexts", strings.clone()),
        ("builder", string.clone()),
        ("cache-from", string_or_strings.clone()),
        ("cache-to", string_or_strings),
        ("cgroup-parent", strings.clone()),
        ("context", string.clone()),
        ("file", string.clone()),
        ("labels", strings.clone()),
        ("load", boolean.clone()),
        ("network", string.clone()),
        ("no-cache", boolean.clone()),
        ("outputs", strings.clone()),
        ("platforms", strings.clone()),
        ("provenance", string.clone()),
        ("pull", boolean.clone()),
        ("push", boolean.clone()),
        ("sbom", boolean),
        ("secrets", strings.clone()),
        ("secret-files", strings.clone()),
        ("shm-size", string.clone()),
        ("ssh", strings.clone()),
        ("tags", strings.clone()),
        ("target", string),
        ("ulimit", strings),
        ("metadata", metadata),
    ];
    let properties: JsonMap = properties
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    json!({"type": "object", "properties": properties})
}

pub fn build(ctx: &PlanCtx<'_>) -> Result<Plan> {
    let task = format!("{}:{}", ctx.project.name, ctx.target);
    let options = combine_options(ctx, &schema())?;
    let spec = plan_build(ctx, &options).map_err(|e| eyre!("{task}: {e:#}"))?;
    let label = spec.label(ctx.env);
    let spec = Arc::new(spec);
    Ok(Plan {
        steps: vec![Step::Native {
            label,
            run: Arc::new(move |nctx: &NativeCtx<'_>| run(&spec, nctx)),
        }],
        parallel: false,
    })
}

// ---------------------------------------------------------------------------
// Plan: inputs and arguments
// ---------------------------------------------------------------------------

/// A slice of an argument. Most are literal; the others are only known when
/// the step runs.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Lit(String),
    /// The executor's temp dir (`docker-build-push-XXXXXX`).
    Tmp,
    /// `new Date().toISOString()` when the metadata is computed.
    Now,
    /// A repository field the GitHub API returns.
    Repo(RepoField),
    /// A builder named by `create-builder` (`<project>-<6 random hex>`).
    Builder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepoField {
    Name,
    Description,
    HtmlUrl,
    License,
}

impl RepoField {
    fn placeholder(self) -> &'static str {
        match self {
            RepoField::Name => "<github:name>",
            RepoField::Description => "<github:description>",
            RepoField::HtmlUrl => "<github:html_url>",
            RepoField::License => "<github:license.spdx_id>",
        }
    }
}

/// When an argument is passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    Always,
    /// `buildx.satisfies(version, '>=a.b.c')`.
    Buildx(u64, u64, u64),
    /// The secret at this index of [`Spec::secrets`] was written.
    SecretFile(usize),
}

#[derive(Clone, Debug)]
struct Arg {
    gate: Gate,
    pieces: Vec<Piece>,
}

enum SecretEntry {
    /// `getSecret` rejected it (`<kvp> is not a valid secret`): warned, skipped.
    Invalid(String),
    /// `getSecretString`: the value (interpolated when written) goes to `file`.
    Value { value: String, file: String },
    /// `getSecretFile`: the file at `path` (interpolated) is copied to `file`,
    /// or the secret is skipped with a warning when it does not exist.
    File { path: String, file: String },
}

struct GithubRepo {
    api_url: String,
    owner: String,
    repo: String,
    token: String,
}

/// Everything the run step needs. Deliberately not `Debug`: it holds secret
/// values.
struct Spec {
    project: String,
    quiet: bool,
    create_builder: bool,
    /// `inputs.builder`; empty with `create_builder` means a generated name.
    builder: String,
    /// The buildx arguments after `buildx`: `build … <context>`.
    args: Vec<Arg>,
    secrets: Vec<SecretEntry>,
    /// Set when the metadata labels need the GitHub repository.
    github: Option<GithubRepo>,
    /// container-metadata's `logger.warn`s, printed where the executor
    /// prints them.
    meta_warnings: Vec<String>,
}

struct Inputs {
    quiet: bool,
    add_hosts: Vec<String>,
    allow: Vec<String>,
    build_args: Vec<String>,
    build_contexts: Vec<String>,
    builder: String,
    cache_from: Vec<String>,
    cache_to: Vec<String>,
    cgroup_parent: String,
    context: String,
    file: String,
    github_token: String,
    labels: Vec<Vec<Piece>>,
    load: bool,
    network: String,
    no_cache: bool,
    no_cache_filters: Vec<String>,
    outputs: Vec<String>,
    platforms: Vec<String>,
    provenance: String,
    pull: bool,
    push: bool,
    sbom: bool,
    secret_files: Vec<String>,
    secrets: Vec<String>,
    shm_size: String,
    ssh: Vec<String>,
    tags: Vec<String>,
    target: String,
    ulimit: Vec<String>,
}

fn plan_build(ctx: &PlanCtx<'_>, options: &JsonMap) -> Result<Spec> {
    let env = ctx.env;
    check_dotenv_config(env, ctx.workspace_root)?;
    let prefix = constant_name(&ctx.project.name);
    let mut inputs = get_inputs(options, &prefix, env, &ctx.project.root)?;

    let engine_fallback = options
        .get("engine")
        .filter(|v| is_truthy(v))
        .cloned()
        .unwrap_or_else(|| json!("docker"));
    let engine = get_input(env, "engine", &prefix, Some(&engine_fallback))?;
    match engine.as_str() {
        "docker" => {}
        "podman" => bail!(
            "engine `podman` is not ported to butler (only the docker engine is); \
             run this target through nx"
        ),
        other => bail!("Unsupported Container Engine `{other}`"),
    }
    let create_builder = to_boolean(&get_input(
        env,
        "create-builder",
        &prefix,
        Some(&json!("false")),
    )?);

    let mut github = None;
    let mut meta_warnings = Vec::new();
    if let Some(metadata) = options.get("metadata")
        && metadata.get("images").is_some_and(is_truthy)
    {
        let meta = get_metadata(metadata, &prefix, env, ctx.workspace_root)?;
        inputs.labels = meta.labels;
        inputs.tags = meta.tags;
        github = meta.github;
        meta_warnings = meta.warnings;
    }

    let (args, secrets) = docker_args(&inputs, create_builder)?;
    Ok(Spec {
        project: ctx.project.name.clone(),
        quiet: inputs.quiet,
        create_builder,
        builder: inputs.builder,
        args,
        secrets,
        github,
        meta_warnings,
    })
}

/// `executor.js` imports `dotenv/config`, which loads the workspace `.env`
/// without overriding: a no-op, since the task environment already holds
/// it. Its environment switches would change that; they are not ported.
fn check_dotenv_config(env: &Env, root: &Path) -> Result<()> {
    if env.contains_key("DOTENV_CONFIG_PATH") {
        bail!("DOTENV_CONFIG_PATH is set: the executor's dotenv/config import is not ported");
    }
    if env
        .get("DOTENV_CONFIG_OVERRIDE")
        .is_some_and(|v| !v.is_empty())
    {
        bail!("DOTENV_CONFIG_OVERRIDE is set: the executor's dotenv/config import is not ported");
    }
    let key = env
        .get("DOTENV_CONFIG_DOTENV_KEY")
        .filter(|v| !v.is_empty())
        .or_else(|| env.get("DOTENV_KEY").filter(|v| !v.is_empty()));
    if key.is_some() && root.join(".env.vault").exists() {
        bail!("DOTENV_KEY with a .env.vault: the executor's dotenv/config import is not ported");
    }
    Ok(())
}

/// `context.getInputs`, fields in the executor's evaluation order (the first
/// failure wins, as there). `load`, `no-cache`, `pull`, `push`, `sbom` and
/// `github-token` read only the unprefixed `INPUT_*` variable, and
/// `github-token` has no option fallback at all — as in the executor.
fn get_inputs(o: &JsonMap, prefix: &str, env: &Env, project_root: &str) -> Result<Inputs> {
    let list = |name: &str, ignore_comma: bool| -> Result<Vec<String>> {
        input_list(
            env,
            name,
            prefix,
            string_list(o.get(name), name)?,
            ignore_comma,
        )
    };
    // `[options[name] ?? []].flat()`
    let flat = |name: &str, ignore_comma: bool| -> Result<Vec<String>> {
        let fallback = match o.get(name) {
            None | Some(Json::Null) => Vec::new(),
            Some(Json::Array(items)) => items
                .iter()
                .map(|i| expect_string(i, name))
                .collect::<Result<_>>()?,
            Some(other) => vec![expect_string(other, name)?],
        };
        input_list(env, name, prefix, fallback, ignore_comma)
    };
    // `getBooleanInput(name, { [prefix,] fallback: `${options[name] || false}` })`
    let boolean = |name: &str, prefixed: bool| -> Result<bool> {
        let fallback = Json::String(match o.get(name) {
            Some(v) if is_truthy(v) => js_string(v),
            _ => "false".into(),
        });
        let prefix = if prefixed { prefix } else { "" };
        Ok(to_boolean(&get_input(env, name, prefix, Some(&fallback))?))
    };
    let string = |name: &str| get_input(env, name, prefix, o.get(name));
    let context_fallback = o
        .get("context")
        .filter(|v| is_truthy(v))
        .cloned()
        .unwrap_or_else(|| json!("."));
    // executor.js: `options.file || join(getProjectRoot(ctx), 'Dockerfile')`.
    // getProjectRoot joins the workspace root, making the path absolute;
    // buildx resolves a relative `--file` against its cwd, the workspace
    // root, so the workspace-relative form names the same file and keeps the
    // label machine-independent.
    let file_fallback = o
        .get("file")
        .filter(|v| is_truthy(v))
        .cloned()
        .unwrap_or_else(|| Json::String(posix_join(project_root, "Dockerfile")));

    Ok(Inputs {
        quiet: boolean("quiet", true)?,
        add_hosts: list("add-hosts", false)?,
        allow: list("allow", false)?,
        build_args: list("build-args", true)?,
        build_contexts: list("build-contexts", true)?,
        builder: string("builder")?,
        cache_from: flat("cache-from", true)?,
        cache_to: flat("cache-to", true)?,
        cgroup_parent: string("cgroup-parent")?,
        context: get_input(env, "context", prefix, Some(&context_fallback))?,
        file: get_input(env, "file", prefix, Some(&file_fallback))?,
        github_token: get_input(env, "github-token", "", None)?,
        labels: list("labels", true)?
            .into_iter()
            .map(|l| vec![Piece::Lit(l)])
            .collect(),
        load: boolean("load", false)?,
        network: string("network")?,
        no_cache: boolean("no-cache", false)?,
        no_cache_filters: list("no-cache-filters", false)?,
        outputs: list("outputs", true)?,
        platforms: list("platforms", false)?,
        provenance: string("provenance")?,
        pull: boolean("pull", false)?,
        push: boolean("push", false)?,
        sbom: boolean("sbom", false)?,
        secret_files: list("secret-files", true)?,
        secrets: list("secrets", true)?,
        shm_size: string("shm-size")?,
        ssh: list("ssh", false)?,
        tags: list("tags", false)?,
        target: string("target")?,
        ulimit: list("ulimit", true)?,
    })
}

#[derive(Default)]
struct ArgList(Vec<Arg>);

impl ArgList {
    fn lit(&mut self, s: &str) {
        self.gated(Gate::Always, vec![Piece::Lit(s.to_string())]);
    }
    fn gated(&mut self, gate: Gate, pieces: Vec<Piece>) {
        self.0.push(Arg { gate, pieces });
    }
    fn pair(&mut self, flag: &str, value: &str) {
        self.lit(flag);
        self.lit(value);
    }
}

/// docker.engine.js `getArgs`: `getBuildArgs`, `getCommonArgs`, context.
fn docker_args(inputs: &Inputs, create_builder: bool) -> Result<(Vec<Arg>, Vec<SecretEntry>)> {
    const DEFAULT_CONTEXT: &str = ".";
    let context = hb_render(&inputs.context, &|name| {
        Ok((name == "defaultContext").then(|| DEFAULT_CONTEXT.to_string()))
    })?;
    let mut a = ArgList::default();
    let mut secrets = Vec::new();
    a.lit("build");
    for h in &inputs.add_hosts {
        a.pair("--add-host", h);
    }
    if !inputs.allow.is_empty() {
        a.pair("--allow", &inputs.allow.join(","));
    }
    for b in &inputs.build_args {
        a.pair("--build-arg", b);
    }
    for b in &inputs.build_contexts {
        let gate = Gate::Buildx(0, 8, 0);
        a.gated(gate, vec![Piece::Lit("--build-context".into())]);
        a.gated(gate, vec![Piece::Lit(b.clone())]);
    }
    for c in &inputs.cache_from {
        a.pair("--cache-from", c);
    }
    for c in &inputs.cache_to {
        a.pair("--cache-to", c);
    }
    if !inputs.cgroup_parent.is_empty() {
        a.pair("--cgroup-parent", &inputs.cgroup_parent);
    }
    if !inputs.file.is_empty() {
        a.pair("--file", &inputs.file);
    }
    if !is_local_or_tar_exporter(&inputs.outputs)? {
        let gate = if inputs.platforms.is_empty() {
            Gate::Always
        } else {
            Gate::Buildx(0, 4, 2)
        };
        a.gated(gate, vec![Piece::Lit("--iidfile".into())]);
        a.gated(gate, vec![Piece::Tmp, Piece::Lit("/iidfile".into())]);
    }
    for l in &inputs.labels {
        a.lit("--label");
        a.gated(Gate::Always, l.clone());
    }
    for f in &inputs.no_cache_filters {
        a.pair("--no-cache-filter", f);
    }
    for o in &inputs.outputs {
        a.pair("--output", o);
    }
    if !inputs.platforms.is_empty() {
        a.pair("--platform", &inputs.platforms.join(","));
    }
    if !inputs.provenance.is_empty() {
        a.pair("--provenance", &inputs.provenance);
    }
    let mut add_secret = |a: &mut ArgList, kvp: &str, from_file: bool| {
        let Some((key, value)) = split_secret(kvp) else {
            secrets.push(SecretEntry::Invalid(format!("{kvp} is not a valid secret")));
            return;
        };
        let index = secrets.len();
        let written = secrets
            .iter()
            .filter(|s| !matches!(s, SecretEntry::Invalid(_)))
            .count();
        let file = format!("secret-{}", written + 1);
        let gate = if from_file {
            Gate::SecretFile(index)
        } else {
            Gate::Always
        };
        a.gated(gate, vec![Piece::Lit("--secret".into())]);
        a.gated(
            gate,
            vec![
                Piece::Lit(format!("id={key},src=")),
                Piece::Tmp,
                Piece::Lit(format!("/{file}")),
            ],
        );
        secrets.push(if from_file {
            SecretEntry::File {
                path: value.to_string(),
                file,
            }
        } else {
            SecretEntry::Value {
                value: value.to_string(),
                file,
            }
        });
    };
    for s in &inputs.secrets {
        add_secret(&mut a, s, false);
    }
    for s in &inputs.secret_files {
        add_secret(&mut a, s, true);
    }
    if !inputs.github_token.is_empty()
        && !inputs
            .secrets
            .iter()
            .any(|s| s.starts_with("GIT_AUTH_TOKEN="))
        && context.starts_with(DEFAULT_CONTEXT)
    {
        add_secret(
            &mut a,
            &format!("GIT_AUTH_TOKEN={}", inputs.github_token),
            false,
        );
    }
    if inputs.sbom {
        a.pair("--attest", "type=sbom");
    }
    if !inputs.shm_size.is_empty() {
        a.pair("--shm-size", &inputs.shm_size);
    }
    for s in &inputs.ssh {
        a.pair("--ssh", s);
    }
    for t in &inputs.tags {
        a.pair("--tag", t);
    }
    if !inputs.target.is_empty() {
        a.pair("--target", &inputs.target);
    }
    for u in &inputs.ulimit {
        a.pair("--ulimit", u);
    }
    // getCommonArgs; `initialize` names the builder before args are built.
    if !inputs.builder.is_empty() {
        a.pair("--builder", &inputs.builder);
    } else if create_builder {
        a.lit("--builder");
        a.gated(Gate::Always, vec![Piece::Builder]);
    }
    if inputs.load {
        a.lit("--load");
    }
    let gate = Gate::Buildx(0, 6, 0);
    a.gated(gate, vec![Piece::Lit("--metadata-file".into())]);
    a.gated(gate, vec![Piece::Tmp, Piece::Lit("/metadata-file".into())]);
    if !inputs.network.is_empty() {
        a.pair("--network", &inputs.network);
    }
    if inputs.no_cache {
        a.lit("--no-cache");
    }
    if inputs.pull {
        a.lit("--pull");
    }
    if inputs.push {
        a.lit("--push");
    }
    a.lit(&context);
    Ok((a.0, secrets))
}

/// buildx.js `getSecret`'s validation: split at the first `=`; both sides
/// must be non-empty (a missing `=` leaves the key empty).
fn split_secret(kvp: &str) -> Option<(&str, &str)> {
    let (key, value) = kvp.split_once('=')?;
    (!key.is_empty() && !value.is_empty()).then_some((key, value))
}

/// buildx.js `isLocalOrTarExporter`.
fn is_local_or_tar_exporter(outputs: &[String]) -> Result<bool> {
    let records = csv_parse(
        &outputs.join("\n"),
        CsvOpts {
            trim: true,
            ..CsvOpts::default()
        },
    )?;
    for record in &records {
        if record.len() == 1 && !record[0].starts_with("type=") {
            return Ok(true);
        }
        for chunk in record {
            let mut parts = chunk.split('=').map(js_trim);
            let key = parts.next().unwrap_or("");
            let value = parts.next();
            if key == "type" && matches!(value, Some("local" | "tar")) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

impl Spec {
    fn builder_label(&self) -> String {
        if self.builder.is_empty() {
            format!("{}-<random>", self.project)
        } else {
            self.builder.clone()
        }
    }

    /// The command(s), temp paths and run-time values as placeholders.
    fn label(&self, env: &Env) -> String {
        let builder = self.builder_label();
        let render_ctx = Render::Label { builder: &builder };
        let mut argv = vec!["docker".to_string(), "buildx".to_string()];
        argv.extend(
            self.args
                .iter()
                .map(|a| interpolate(&render(&a.pieces, &render_ctx), env)),
        );
        let build = shell_join(&argv);
        if !self.create_builder {
            return build;
        }
        let create = shell_join(&[
            "docker".into(),
            "buildx".into(),
            "create".into(),
            format!("--name={builder}"),
        ]);
        let rm = shell_join(&["docker".into(), "buildx".into(), "rm".into(), builder]);
        format!("{create} && {build} && {rm}")
    }
}

enum Render<'a> {
    Label {
        builder: &'a str,
    },
    Run {
        tmp: &'a str,
        now: &'a str,
        repo: &'a RepoValues,
        builder: &'a str,
    },
}

fn render(pieces: &[Piece], r: &Render<'_>) -> String {
    let mut out = String::new();
    for p in pieces {
        match (p, r) {
            (Piece::Lit(s), _) => out.push_str(s),
            (Piece::Tmp, Render::Label { .. }) => out.push_str("<tmp>"),
            (Piece::Now, Render::Label { .. }) => out.push_str("<now>"),
            (Piece::Repo(f), Render::Label { .. }) => out.push_str(f.placeholder()),
            (Piece::Builder, Render::Label { builder }) => out.push_str(builder),
            (Piece::Tmp, Render::Run { tmp, .. }) => out.push_str(tmp),
            (Piece::Now, Render::Run { now, .. }) => out.push_str(now),
            (Piece::Repo(f), Render::Run { repo, .. }) => out.push_str(repo.get(*f)),
            (Piece::Builder, Render::Run { builder, .. }) => out.push_str(builder),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// @nx-tools/core: inputs, lists, interpolation
// ---------------------------------------------------------------------------

/// `getInput(name, { prefix, fallback })`: `INPUT_<PREFIX>_<NAME>`, then
/// `INPUT_<NAME>` (empty counts as unset), then a truthy fallback; trimmed.
/// The executor calls `.trim()` on the fallback, so a non-string one crashes
/// it — here it is an error.
fn get_input(env: &Env, name: &str, prefix: &str, fallback: Option<&Json>) -> Result<String> {
    let mut val = String::new();
    if !prefix.is_empty() {
        val = env
            .get(&posix_name(&format!("{prefix}_{name}")))
            .cloned()
            .unwrap_or_default();
    }
    if val.is_empty() {
        val = env.get(&posix_name(name)).cloned().unwrap_or_default();
    }
    if val.is_empty()
        && let Some(f) = fallback.filter(|f| is_truthy(f))
    {
        match f {
            Json::String(s) => val = s.clone(),
            other => bail!(
                "option `{name}` must be a string (the executor calls .trim() on it), got {other}"
            ),
        }
    }
    Ok(js_trim(&val).to_string())
}

/// core `getPosixName`: `names('input-' + name.toLowerCase()).constantName`.
fn posix_name(name: &str) -> String {
    constant_name(&format!("input-{}", name.to_lowercase()))
}

/// core `toBoolean` for strings.
fn to_boolean(s: &str) -> bool {
    matches!(
        js_trim(s).to_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

/// nx-container's own `getInputList` (context.js): an `INPUT_*` value is
/// parsed as CSV — each line one item, a multi-field line split into items
/// or, with `ignore_comma`, re-joined — then emptied items dropped and the
/// rest trimmed. Without the variable the option's list is used verbatim.
fn input_list(
    env: &Env,
    name: &str,
    prefix: &str,
    fallback: Vec<String>,
    ignore_comma: bool,
) -> Result<Vec<String>> {
    let items = get_input(env, name, prefix, None)?;
    if items.is_empty() {
        return Ok(fallback);
    }
    let records = csv_parse(
        &items,
        CsvOpts {
            relax_quotes: true,
            skip_empty_lines: true,
            ..CsvOpts::default()
        },
    )?;
    let mut res = Vec::new();
    for record in records {
        if record.len() == 1 || ignore_comma {
            res.push(record.join(","));
        } else {
            res.extend(record);
        }
    }
    Ok(clean_list(res))
}

/// container-metadata's inputs: core `getInputList(name, { prefix, fallback,
/// ignoreComma: true, comment: '#' })`, each item then interpolated.
fn meta_input_list(
    env: &Env,
    name: &str,
    prefix: &str,
    fallback: Option<&Json>,
) -> Result<Vec<String>> {
    let input = get_input(env, name, prefix, None)?;
    let list = if input.is_empty() {
        string_list(fallback, name)?
    } else {
        let records = csv_parse(
            &input,
            CsvOpts {
                comment: Some('#'),
                relax_quotes: true,
                skip_empty_lines: true,
                ..CsvOpts::default()
            },
        )?;
        clean_list(records.into_iter().map(|r| r.join(",")).collect())
    };
    Ok(list.iter().map(|s| interpolate(s, env)).collect())
}

fn clean_list(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .filter(|s| !s.is_empty())
        .map(|s| js_trim(&s).to_string())
        .collect()
}

/// An option list (`fallback ?? []`); items must be strings — the executor
/// would crash interpolating anything else.
fn string_list(v: Option<&Json>, name: &str) -> Result<Vec<String>> {
    match v {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Array(items)) => items.iter().map(|i| expect_string(i, name)).collect(),
        Some(other) => bail!("option `{name}` must be an array, got {other}"),
    }
}

fn expect_string(v: &Json, name: &str) -> Result<String> {
    match v {
        Json::String(s) => Ok(s.clone()),
        other => bail!("option `{name}` must hold strings, got {other}"),
    }
}

/// core `interpolate`: `/\${?([a-zA-Z0-9_]+)?}?/g` → `process.env[name] ||
/// match`, so unset and empty variables stay literal (and `${}` looks up
/// `undefined`, as the JavaScript does).
fn interpolate(s: &str, env: &Env) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut last) = (0, 0);
    while i < b.len() {
        if b[i] != b'$' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if b.get(j) == Some(&b'{') {
            j += 1;
        }
        let name_start = j;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        let name = &s[name_start..j];
        if b.get(j) == Some(&b'}') {
            j += 1;
        }
        let key = if name.is_empty() { "undefined" } else { name };
        out.push_str(&s[last..i]);
        match env.get(key).filter(|v| !v.is_empty()) {
            Some(v) => out.push_str(v),
            None => out.push_str(&s[i..j]),
        }
        last = j;
        i = j;
    }
    out.push_str(&s[last..]);
    out
}

/// nx devkit `names(s).constantName`.
fn constant_name(s: &str) -> String {
    let normalized = if s.to_uppercase() == s {
        s.to_lowercase()
    } else {
        s.to_string()
    };
    file_name(&property_name(&normalized))
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .to_uppercase()
}

/// nx devkit `toPropertyName`: a run of non-alphanumerics upper-cases the
/// character after it and disappears; a leading capital is lowered.
fn property_name(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphanumeric() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        while i < chars.len() && !chars[i].is_ascii_alphanumeric() {
            i += 1;
        }
        if let Some(c) = chars.get(i) {
            out.extend(c.to_uppercase());
            i += 1;
        }
    }
    let mut out: String = out.chars().filter(char::is_ascii_alphanumeric).collect();
    if out.starts_with(|c: char| c.is_ascii_uppercase()) {
        out[..1].make_ascii_lowercase();
    }
    out
}

/// nx devkit `toFileName`.
fn file_name(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut split = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if (c.is_ascii_lowercase() || c.is_ascii_digit())
            && chars.get(i + 1).is_some_and(char::is_ascii_uppercase)
        {
            split.push(c);
            split.push('_');
            split.push(chars[i + 1]);
            i += 2;
        } else {
            split.push(c);
            i += 1;
        }
    }
    split
        .to_lowercase()
        .chars()
        .enumerate()
        .map(|(i, c)| match c {
            '_' if i == 0 => '_',
            ' ' | '_' => '-',
            c => c,
        })
        .collect()
}

/// JavaScript truthiness of a JSON value.
fn is_truthy(v: &Json) -> bool {
    match v {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Json::String(s) => !s.is_empty(),
        Json::Array(_) | Json::Object(_) => true,
    }
}

/// JavaScript `WhiteSpace` + `LineTerminator` (what `String.prototype.trim`
/// and regex `\s` match; csv-parse's trimmable characters are the same set).
fn is_js_space(c: char) -> bool {
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
    s.trim_matches(is_js_space)
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// JavaScript `Number(string)`.
fn js_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    for (p, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = t.strip_prefix(p) {
            if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                return f64::NAN;
            }
            return digits.chars().fold(0.0, |acc, c| {
                acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
            });
        }
    }
    if t.chars()
        .any(|c| !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')))
    {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

/// node `path.posix.join(root, name)` for a workspace-relative root.
fn posix_join(root: &str, name: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in root.split('/').chain([name]) {
        match seg {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let joined = parts.join("/");
    if root.starts_with('/') {
        format!("/{joined}")
    } else {
        joined
    }
}

// ---------------------------------------------------------------------------
// csv-parse 7.0.2 (`csv-parse/sync`), for the option sets the executor uses
// ---------------------------------------------------------------------------

/// Delimiter `,`, quote and escape `"`, record delimiter auto-detected,
/// `relax_column_count` on (every caller sets it). The state machine is the
/// library's; the special characters are all ASCII, so walking chars is
/// walking its bytes.
#[derive(Clone, Copy, Default)]
struct CsvOpts {
    comment: Option<char>,
    relax_quotes: bool,
    skip_empty_lines: bool,
    /// `trim: true` (ltrim + rtrim).
    trim: bool,
}

fn csv_parse(input: &str, o: CsvOpts) -> Result<Vec<Vec<String>>> {
    const DELIMS: [&[char]; 3] = [&['\r', '\n'], &['\n'], &['\r']];
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let at = |pos: usize, d: &[char]| chars.get(pos..).is_some_and(|s| s.starts_with(d));
    let discover = |pos: usize| DELIMS.into_iter().find(|d| at(pos, d));
    let trimmable = |pos: usize| chars.get(pos).copied().is_some_and(is_js_space);
    let fail = |msg: String| eyre!("csv-parse: {msg} in {input:?}");

    let mut rd: Option<&[char]> = None;
    let (mut quoting, mut was_quoting, mut commenting, mut escaping) = (false, false, false, false);
    let mut field = String::new();
    let mut record: Vec<String> = Vec::new();
    let mut records = Vec::new();
    let end_field = |field: &mut String, record: &mut Vec<String>, was_quoting: &mut bool| {
        let f = if o.trim && !*was_quoting {
            field.trim_end_matches(is_js_space).to_string()
        } else {
            field.clone()
        };
        record.push(f);
        field.clear();
        *was_quoting = false;
    };

    let mut pos = 0;
    'chars: while pos < n {
        if !quoting && rd.is_none() {
            rd = discover(pos);
        }
        let chr = chars[pos];
        'special: {
            if escaping {
                escaping = false;
                break 'special;
            }
            if quoting && chr == '"' && pos + 1 < n && chars[pos + 1] == '"' {
                escaping = true;
                pos += 1;
                continue 'chars;
            }
            if !commenting && chr == '"' {
                if quoting {
                    let next = chars.get(pos + 1).copied();
                    let next_rd = match rd {
                        Some(d) => at(pos + 1, d),
                        None => {
                            rd = discover(pos + 1);
                            rd.is_some()
                        }
                    };
                    if matches!(next, None | Some('\0') | Some(','))
                        || next_rd
                        || (o.comment.is_some() && next == o.comment)
                        || (o.trim && trimmable(pos + 1))
                    {
                        quoting = false;
                        was_quoting = true;
                        pos += 1;
                        continue 'chars;
                    }
                    if !o.relax_quotes {
                        return Err(fail(format!(
                            "Invalid Closing Quote: got {:?} instead of delimiter, record \
                             delimiter, trimable character (if activated) or comment",
                            next.unwrap_or_default()
                        )));
                    }
                    quoting = false;
                    was_quoting = true;
                    field.insert(0, '"');
                } else if !field.is_empty() {
                    if !o.relax_quotes {
                        return Err(fail(format!(
                            "Invalid Opening Quote: a quote is found on field {}, value is {field:?}",
                            record.len()
                        )));
                    }
                } else {
                    quoting = true;
                    pos += 1;
                    continue 'chars;
                }
            }
            if !quoting {
                if let Some(d) = rd.filter(|d| at(pos, d)) {
                    let comment_line =
                        commenting && !was_quoting && record.is_empty() && field.is_empty();
                    if !comment_line {
                        if o.skip_empty_lines
                            && !was_quoting
                            && record.is_empty()
                            && field.is_empty()
                        {
                            pos += d.len();
                            continue 'chars;
                        }
                        end_field(&mut field, &mut record, &mut was_quoting);
                        records.push(std::mem::take(&mut record));
                    }
                    commenting = false;
                    pos += d.len();
                    continue 'chars;
                }
                if commenting {
                    pos += 1;
                    continue 'chars;
                }
                if o.comment == Some(chr) {
                    commenting = true;
                    pos += 1;
                    continue 'chars;
                }
                if chr == ',' {
                    end_field(&mut field, &mut record, &mut was_quoting);
                    pos += 1;
                    continue 'chars;
                }
            }
        }
        let lappend = !o.trim || quoting || !field.is_empty() || !trimmable(pos);
        let rappend = !o.trim || !was_quoting;
        if lappend && rappend {
            field.push(chr);
        } else if o.trim && !trimmable(pos) {
            return Err(fail(
                "Invalid Closing Quote: found non trimable byte after quote".into(),
            ));
        }
        pos += 1;
    }
    if quoting {
        return Err(fail(
            "Quote Not Closed: the parsing is finished with an opening quote".into(),
        ));
    }
    if was_quoting || !record.is_empty() || !field.is_empty() {
        end_field(&mut field, &mut record, &mut was_quoting);
        records.push(record);
    }
    Ok(records)
}

/// `parse(s, { relaxColumnCount: true, skipEmptyLines: true })[0]`, as the
/// tag/image/flavor parsers use it (an empty entry crashes them).
fn first_record(s: &str, what: &str) -> Result<Vec<String>> {
    csv_parse(
        s,
        CsvOpts {
            skip_empty_lines: true,
            ..CsvOpts::default()
        },
    )?
    .into_iter()
    .next()
    .ok_or_else(|| eyre!("empty {what} entry `{s}`"))
}

// ---------------------------------------------------------------------------
// handlebars, the subset the executor's templates can use
// ---------------------------------------------------------------------------

/// Plain `{{name}}` / `{{{name}}}` / `{{&name}}` references to the helpers
/// and values the executor provides. Blocks, parameters, paths, literals,
/// comments, whitespace control and `\{{` escapes are refused: they are not
/// ported, and approximating them would change tags silently.
enum Hb {
    Text(String),
    Var { name: String, escape: bool },
}

fn hb_parse(t: &str) -> Result<Vec<Hb>> {
    let unsupported = |what: &str| {
        eyre!(
            "unsupported handlebars expression `{what}` in `{t}` (butler's port renders \
             plain {{{{name}}}} references only; run this target through nx)"
        )
    };
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(i) = rest.find("{{") {
        let text = &rest[..i];
        if text.ends_with('\\') {
            return Err(unsupported("\\{{"));
        }
        if !text.is_empty() {
            out.push(Hb::Text(text.to_string()));
        }
        let after = &rest[i + 2..];
        let (inner, escape, next) = if let Some(a) = after.strip_prefix('{') {
            let e = a.find("}}}").ok_or_else(|| unsupported("{{{"))?;
            (&a[..e], false, &a[e + 3..])
        } else {
            let e = after.find("}}").ok_or_else(|| unsupported("{{"))?;
            match after[..e].strip_prefix('&') {
                Some(inner) => (inner, false, &after[e + 2..]),
                None => (&after[..e], true, &after[e + 2..]),
            }
        };
        let name = inner.trim_matches(is_js_space);
        if !is_hb_id(name) {
            return Err(unsupported(inner));
        }
        out.push(Hb::Var {
            name: name.to_string(),
            escape,
        });
        rest = next;
    }
    if !rest.is_empty() {
        out.push(Hb::Text(rest.to_string()));
    }
    Ok(out)
}

/// A handlebars `ID` token that is a simple lookup (not a literal or `this`).
fn is_hb_id(name: &str) -> bool {
    let number = {
        let d = name.strip_prefix('-').unwrap_or(name);
        !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())
    };
    !name.is_empty()
        && !number
        && !matches!(
            name,
            "true" | "false" | "null" | "undefined" | "this" | "else"
        )
        && name
            .chars()
            .all(|c| !is_js_space(c) && !"!\"#%&'()*+,./;<=>@[\\]^`{|}~".contains(c))
}

/// `handlebars.compile(t)(context)`: `lookup` yields a context value, `None`
/// for a name the context does not have (rendered empty, as handlebars
/// does), or an error for a helper that fails.
fn hb_render(t: &str, lookup: &dyn Fn(&str) -> Result<Option<String>>) -> Result<String> {
    let mut out = String::new();
    for node in hb_parse(t)? {
        match node {
            Hb::Text(s) => out.push_str(&s),
            Hb::Var { name, escape } => {
                if let Some(v) = lookup(&name)? {
                    out.push_str(&if escape { escape_expression(&v) } else { v });
                }
            }
        }
    }
    Ok(out)
}

/// handlebars `escapeExpression`.
fn escape_expression(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            '=' => out.push_str("&#x3D;"),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// @nx-tools/ci-context: where sha/ref/event come from
// ---------------------------------------------------------------------------

/// A ci-context `ContextProxyFactory` result (the fields metadata reads).
/// `None` is JavaScript `undefined`.
struct CiContext {
    event_name: Option<String>,
    git_ref: Option<String>,
    sha: Option<String>,
    payload: Json,
}

impl CiContext {
    /// `/^prefix/.test(context.ref)` (`undefined` never matches).
    fn ref_starts(&self, prefix: &str) -> bool {
        self.git_ref
            .as_deref()
            .is_some_and(|r| r.starts_with(prefix))
    }
    /// `context.ref.replace(/^prefix/g, '')`, for a ref known to match.
    fn ref_without(&self, prefix: &str) -> String {
        let r = self.git_ref.as_deref().unwrap_or_default();
        r.strip_prefix(prefix).unwrap_or(r).to_string()
    }
    fn event_matches(&self, needle: &str) -> bool {
        self.event_name
            .as_deref()
            .unwrap_or("undefined")
            .contains(needle)
    }
}

/// A ci-context `RepoProxyFactory` result: known at plan time, or fetched
/// from the GitHub API when the step runs.
enum Repo {
    Known(KnownRepo),
    GitHub,
}

#[derive(Default)]
struct KnownRepo {
    default_branch: Option<String>,
    description: Option<String>,
    html_url: Option<String>,
    name: Option<String>,
}

/// Repository values as the labels use them (`repo.x || ''`).
#[derive(Default)]
struct RepoValues {
    name: String,
    description: String,
    html_url: String,
    license: String,
}

impl RepoValues {
    fn get(&self, f: RepoField) -> &str {
        match f {
            RepoField::Name => &self.name,
            RepoField::Description => &self.description,
            RepoField::HtmlUrl => &self.html_url,
            RepoField::License => &self.license,
        }
    }
}

/// std-env 3.10.0 `provider`: the first entry whose variable (the second
/// element, else the name) is set and non-empty.
const PROVIDERS: &[(&str, Option<&str>)] = &[
    ("APPVEYOR", None),
    ("AWS_AMPLIFY", Some("AWS_APP_ID")),
    (
        "AZURE_PIPELINES",
        Some("SYSTEM_TEAMFOUNDATIONCOLLECTIONURI"),
    ),
    (
        "AZURE_STATIC",
        Some("INPUT_AZURE_STATIC_WEB_APPS_API_TOKEN"),
    ),
    ("APPCIRCLE", Some("AC_APPCIRCLE")),
    ("BAMBOO", Some("bamboo_planKey")),
    ("BITBUCKET", Some("BITBUCKET_COMMIT")),
    ("BITRISE", Some("BITRISE_IO")),
    ("BUDDY", Some("BUDDY_WORKSPACE_ID")),
    ("BUILDKITE", None),
    ("CIRCLE", Some("CIRCLECI")),
    ("CIRRUS", Some("CIRRUS_CI")),
    ("CLOUDFLARE_PAGES", Some("CF_PAGES")),
    ("CLOUDFLARE_WORKERS", Some("WORKERS_CI")),
    ("CODEBUILD", Some("CODEBUILD_BUILD_ARN")),
    ("CODEFRESH", Some("CF_BUILD_ID")),
    ("DRONE", None),
    ("DRONE", Some("DRONE_BUILD_EVENT")),
    ("DSARI", None),
    ("GITHUB_ACTIONS", None),
    ("GITLAB", Some("GITLAB_CI")),
    ("GITLAB", Some("CI_MERGE_REQUEST_ID")),
    ("GOCD", Some("GO_PIPELINE_LABEL")),
    ("LAYERCI", None),
    ("HUDSON", Some("HUDSON_URL")),
    ("JENKINS", Some("JENKINS_URL")),
    ("MAGNUM", None),
    ("NETLIFY", None),
    ("NETLIFY", Some("NETLIFY_LOCAL")),
    ("NEVERCODE", None),
    ("RENDER", None),
    ("SAIL", Some("SAILCI")),
    ("SEMAPHORE", None),
    ("SCREWDRIVER", None),
    ("SHIPPABLE", None),
    ("SOLANO", Some("TDDIUM")),
    ("STRIDER", None),
    ("TEAMCITY", Some("TEAMCITY_VERSION")),
    ("TRAVIS", None),
    ("VERCEL", Some("NOW_BUILDER")),
    ("VERCEL", Some("VERCEL")),
    ("VERCEL", Some("VERCEL_ENV")),
    ("APPCENTER", Some("APPCENTER_BUILD_ID")),
    ("CODESANDBOX", Some("CODESANDBOX_SSE")),
    ("CODESANDBOX", Some("CODESANDBOX_HOST")),
    ("STACKBLITZ", None),
    ("STORMKIT", None),
    ("CLEAVR", None),
    ("ZEABUR", None),
    ("CODESPHERE", Some("CODESPHERE_APP_ID")),
    ("RAILWAY", Some("RAILWAY_PROJECT_ID")),
    ("RAILWAY", Some("RAILWAY_SERVICE_ID")),
    ("DENO-DEPLOY", Some("DENO_DEPLOYMENT_ID")),
    ("FIREBASE_APP_HOSTING", Some("FIREBASE_APP_HOSTING")),
];

fn ci_provider(env: &Env) -> String {
    PROVIDERS
        .iter()
        .find(|(name, var)| env.get(var.unwrap_or(name)).is_some_and(|v| !v.is_empty()))
        .map(|(name, _)| name.to_lowercase())
        .unwrap_or_default()
}

fn env_opt(env: &Env, key: &str) -> Option<String> {
    env.get(key).cloned()
}

fn env_truthy<'a>(env: &'a Env, key: &str) -> Option<&'a str> {
    env.get(key).map(String::as_str).filter(|v| !v.is_empty())
}

/// `` `refs/tags/${TAG}` `` when the tag variable is set, else
/// `` `refs/heads/${BRANCH}` `` (an unset branch renders `undefined`).
fn tag_or_branch_ref(env: &Env, tag: &str, branch: &str) -> Option<String> {
    Some(match env_truthy(env, tag) {
        Some(t) => format!("refs/tags/{t}"),
        None => format!(
            "refs/heads/{}",
            env.get(branch).map_or("undefined", String::as_str)
        ),
    })
}

fn pr_or_unknown(env: &Env, key: &str) -> Option<String> {
    Some(
        if env_truthy(env, key).is_some() {
            "pull_request"
        } else {
            "unknown"
        }
        .into(),
    )
}

fn ci_context(provider: &str, env: &Env, git: &Git<'_>, payload: &Json) -> Result<CiContext> {
    let ctx = |event_name, git_ref, sha, payload| CiContext {
        event_name,
        git_ref,
        sha,
        payload,
    };
    Ok(match provider {
        "azure_pipelines" => ctx(
            pr_or_unknown(env, "SYSTEM_PULLREQUEST_PULLREQUESTID"),
            env_truthy(env, "SYSTEM_PULLREQUEST_SOURCEBRANCH")
                .map(str::to_string)
                .or_else(|| env_opt(env, "BUILD_SOURCEBRANCH")),
            env_opt(env, "BUILD_SOURCEVERSION"),
            json!({}),
        ),
        "bitbucket" => ctx(
            pr_or_unknown(env, "BITBUCKET_PR_ID"),
            tag_or_branch_ref(env, "BITBUCKET_TAG", "BITBUCKET_BRANCH"),
            env_opt(env, "BITBUCKET_COMMIT"),
            json!({}),
        ),
        "circle" => ctx(
            pr_or_unknown(env, "CI_PULL_REQUEST"),
            tag_or_branch_ref(env, "CIRCLE_TAG", "CIRCLE_BRANCH"),
            env_opt(env, "CIRCLE_SHA1"),
            json!({}),
        ),
        "drone" => ctx(
            env_opt(env, "DRONE_BUILD_EVENT"),
            env_opt(env, "DRONE_COMMIT_REF"),
            env_opt(env, "DRONE_COMMIT_SHA"),
            json!({"repository": {
                "private": env.get("DRONE_REPO_PRIVATE").is_some_and(|v| v == "true")
            }}),
        ),
        // `?? ''`: an unset variable reads as empty, not undefined.
        "github_actions" => ctx(
            Some(env_opt(env, "GITHUB_EVENT_NAME").unwrap_or_default()),
            Some(env_opt(env, "GITHUB_REF").unwrap_or_default()),
            Some(env_opt(env, "GITHUB_SHA").unwrap_or_default()),
            payload.clone(),
        ),
        "gitlab" => ctx(
            env_opt(env, "CI_PIPELINE_SOURCE"),
            match env_truthy(env, "CI_COMMIT_TAG") {
                Some(t) => Some(format!("refs/tags/{t}")),
                None => Some(format!(
                    "refs/heads/{}",
                    env.get("CI_COMMIT_REF_SLUG")
                        .map_or("undefined", String::as_str)
                )),
            },
            env_opt(env, "CI_COMMIT_SHA"),
            json!({"repository": {
                "default_branch": env.get("CI_DEFAULT_BRANCH"),
                "private": env.get("CI_PROJECT_VISIBILITY").is_some_and(|v| v == "private")
            }}),
        ),
        "hudson"
            if env_truthy(env, "JENKINS")
                .or(env_truthy(env, "JENKINS_URL"))
                .is_some() =>
        {
            jenkins_context(env)
        }
        "jenkins" => jenkins_context(env),
        "semaphore" => ctx(
            env_opt(env, "SEMAPHORE_GIT_REF_TYPE"),
            env_opt(env, "SEMAPHORE_GIT_REF"),
            env_opt(env, "SEMAPHORE_GIT_SHA"),
            json!({}),
        ),
        "travis" => ctx(
            env_opt(env, "TRAVIS_EVENT_TYPE"),
            tag_or_branch_ref(env, "TRAVIS_TAG", "TRAVIS_BRANCH"),
            env_opt(env, "TRAVIS_COMMIT"),
            json!({}),
        ),
        "teamcity" => bail!(
            "CI provider TeamCity is not ported (ci-context reads its build properties files); \
             run this target through nx"
        ),
        _ => git.context()?,
    })
}

fn jenkins_context(env: &Env) -> CiContext {
    CiContext {
        event_name: Some(
            env_truthy(env, "CHANGE_FORK")
                .unwrap_or("unknown")
                .to_string(),
        ),
        git_ref: tag_or_branch_ref(env, "TAG_NAME", "BRANCH_NAME"),
        sha: env_opt(env, "GIT_COMMIT"),
        payload: json!({}),
    }
}

fn ci_repo(
    provider: &str,
    env: &Env,
    git: &Git<'_>,
    token: &str,
    payload: &Json,
) -> Result<(Repo, Option<GithubRepo>)> {
    let known = |default_branch: Option<String>, html_url: Option<String>, name: Option<String>| {
        Repo::Known(KnownRepo {
            default_branch,
            description: Some(String::new()),
            html_url,
            name,
        })
    };
    let empty = || Some(String::new());
    Ok((
        match provider {
            "azure_pipelines" => known(
                empty(),
                env_opt(env, "BUILD_REPOSITORY_URI"),
                env_opt(env, "AGENT_JOBNAME"),
            ),
            "bitbucket" => known(
                empty(),
                Some(format!(
                    "https://bitbucket.org/{}",
                    env.get("BITBUCKET_REPO_FULL_NAME")
                        .map_or("undefined", String::as_str)
                )),
                env_opt(env, "BITBUCKET_WORKSPACE"),
            ),
            "circle" => known(
                empty(),
                env_opt(env, "CIRCLE_REPOSITORY_URL"),
                env_opt(env, "CIRCLE_PROJECT_REPONAME"),
            ),
            "drone" => known(
                env_opt(env, "DRONE_REPO_BRANCH"),
                env_opt(env, "DRONE_REPO_LINK"),
                env_opt(env, "DRONE_REPO"),
            ),
            "github_actions" => {
                return Ok((Repo::GitHub, Some(github_repo(env, token, payload)?)));
            }
            "gitlab" => known(
                env_opt(env, "CI_DEFAULT_BRANCH"),
                env_opt(env, "CI_PROJECT_URL"),
                env_opt(env, "CI_PROJECT_NAME"),
            ),
            "jenkins" => known(
                empty(),
                env_opt(env, "GIT_URL"),
                env_opt(env, "JOB_BASE_NAME"),
            ),
            "semaphore" => {
                let slug = env
                    .get("SEMAPHORE_GIT_REPO_SLUG")
                    .ok_or_else(|| eyre!("SEMAPHORE_GIT_REPO_SLUG is not set"))?;
                known(
                    empty(),
                    env_opt(env, "SEMAPHORE_GIT_URL"),
                    slug.split('/').nth(1).map(str::to_string),
                )
            }
            "travis" => known(empty(), empty(), env_opt(env, "TRAVIS_REPO_SLUG")),
            _ => known(empty(), Some(git.remote_url()?), empty()),
        },
        None,
    ))
}

/// ci-context's GitHub `repo(token)`: `octokit.rest.repos.get` for
/// `@actions/github`'s `context.repo`. The request itself happens when the
/// step runs.
fn github_repo(env: &Env, token: &str, payload: &Json) -> Result<GithubRepo> {
    if token.is_empty() {
        bail!("Missing github token");
    }
    let (owner, repo) = if let Some(full) = env_truthy(env, "GITHUB_REPOSITORY") {
        let mut parts = full.split('/');
        let owner = parts.next().unwrap_or_default().to_string();
        let repo = parts
            .next()
            .ok_or_else(|| eyre!("GITHUB_REPOSITORY `{full}` is not `owner/repo`"))?;
        (owner, repo.to_string())
    } else if let Some(r) = payload.get("repository").filter(|r| is_truthy(r)) {
        let field = |v: Option<&Json>, what: &str| {
            v.and_then(Json::as_str)
                .map(str::to_string)
                .ok_or_else(|| eyre!("the event payload has no {what}"))
        };
        (
            field(
                r.get("owner").and_then(|o| o.get("login")),
                "repository.owner.login",
            )?,
            field(r.get("name"), "repository.name")?,
        )
    } else {
        bail!("context.repo requires a GITHUB_REPOSITORY environment variable like 'owner/repo'");
    };
    Ok(GithubRepo {
        api_url: env_truthy(env, "GITHUB_API_URL")
            .unwrap_or("https://api.github.com")
            .to_string(),
        owner,
        repo,
        token: token.to_string(),
    })
}

/// `@actions/github`'s context, constructed when container-metadata is
/// imported: it parses `GITHUB_EVENT_PATH` when that file exists (invalid
/// JSON fails the import); ci-context's GitHub context reads the same file.
fn actions_github_payload(env: &Env, root: &Path) -> Result<Json> {
    let Some(path) = env_truthy(env, "GITHUB_EVENT_PATH") else {
        return Ok(json!({}));
    };
    let full = root.join(path);
    if !full.exists() {
        return Ok(json!({}));
    }
    let text = fs::read(&full).map_err(|e| eyre!("read {path}: {e}"))?;
    serde_json::from_str(&String::from_utf8_lossy(&text))
        .map_err(|e| eyre!("GITHUB_EVENT_PATH {path}: {e}"))
}

/// ci-context's local-git fallback (`utils/git.js`), run read-only at plan
/// time with the environment core `exec` gives it.
struct Git<'a> {
    root: &'a Path,
    env: Env,
}

impl Git<'_> {
    fn exec(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(self.root)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| eyre!("git {}: {e}", args.join(" ")))?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.is_empty() && out.status.code() != Some(0) {
            bail!("git {}: {}", args.join(" "), stderr.trim_end());
        }
        Ok(js_trim(&String::from_utf8_lossy(&out.stdout)).to_string())
    }

    fn context(&self) -> Result<CiContext> {
        // `actor` is only logged, but a failing `git log` fails the executor.
        self.exec(&["log", "-1", "--pretty=format:%ae"])?;
        let git_ref = if self.exec(&["branch", "--show-current"])?.is_empty() {
            detached_ref(&self.exec(&["show", "-s", "--pretty=%D"])?)?
        } else {
            self.exec(&["symbolic-ref", "HEAD"])?
        };
        self.remote_url()?;
        let sha = self.exec(&["show", "--format=%H", "HEAD", "--quiet", "--"])?;
        Ok(CiContext {
            event_name: Some("push".into()),
            git_ref: Some(git_ref),
            sha: Some(sha),
            payload: json!({}),
        })
    }

    fn remote_url(&self) -> Result<String> {
        let origin = self.exec(&["remote", "get-url", "origin"])?;
        if !origin.is_empty() {
            return Ok(origin);
        }
        let upstream = self.exec(&["remote", "get-url", "upstream"])?;
        if upstream.is_empty() {
            bail!("Cannot find remote URL for origin or upstream");
        }
        Ok(upstream)
    }
}

/// git.js `getDetachedRef` on `git show -s --pretty=%D` output.
fn detached_ref(res: &str) -> Result<String> {
    let rest = res
        .strip_prefix("grafted, ")
        .and_then(|r| r.strip_prefix("HEAD, "))
        .or_else(|| res.strip_prefix("HEAD, "))
        .filter(|r| !r.is_empty() && !r.contains(is_line_terminator))
        .ok_or_else(|| eyre!("Cannot find detached HEAD ref in \"{res}\""))?;
    let r = js_trim(rest);
    if r.starts_with("tag: ") {
        return Ok(format!(
            "refs/tags/{}",
            js_trim(r.split(':').nth(1).unwrap_or_default())
        ));
    }
    // `/^[^/]+\/[^/]+, (.+)$/`: the greediest `, ` before a second `/`.
    if let Some(slash) = r.find('/').filter(|s| *s > 0) {
        let after = &r[slash + 1..];
        let limit = after.find('/').unwrap_or(after.len());
        let candidates: Vec<usize> = after[..limit]
            .match_indices(", ")
            .map(|(j, _)| j)
            .filter(|j| *j > 0)
            .collect();
        let branch = candidates
            .into_iter()
            .rev()
            .map(|j| &after[j + 2..])
            .find(|cap| !cap.is_empty() && !cap.contains(is_line_terminator));
        if let Some(b) = branch {
            return Ok(format!("refs/heads/{}", js_trim(b)));
        }
    }
    if let Some(pr) = r.strip_prefix("pull/")
        && let Some((num, kind)) = pr.split_once('/')
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
        && matches!(kind, "head" | "merge")
    {
        return Ok(format!("refs/{r}"));
    }
    bail!("Unsupported detached HEAD ref in \"{res}\"")
}

// ---------------------------------------------------------------------------
// @nx-tools/container-metadata: images, tags, flavor, version, labels
// ---------------------------------------------------------------------------

struct MetaOut {
    tags: Vec<String>,
    labels: Vec<Vec<Piece>>,
    github: Option<GithubRepo>,
    warnings: Vec<String>,
}

/// `getMetadata({ ...options.metadata, quiet }, ctx)`: its tags and labels,
/// which replace the executor's `tags`/`labels`. The bake files and the
/// annotations/JSON outputs it also produces are only logged or written to
/// an unreferenced temp dir, so they are not reproduced.
fn get_metadata(opts: &Json, prefix: &str, env: &Env, root: &Path) -> Result<MetaOut> {
    let payload = actions_github_payload(env, root)?;
    let token = get_input(env, "github-token", "", None)?;
    let list = |name: &str| meta_input_list(env, name, prefix, opts.get(name));
    // Parsed for its errors only: annotations feed outputs the executor ignores.
    list("annotations")?;
    let flavor = list("flavor")?;
    let images = list("images")?;
    let labels = list("labels")?;
    let tags = list("tags")?;

    let provider = ci_provider(env);
    let git = Git {
        root,
        env: exec_env(root, env),
    };
    let context = ci_context(&provider, env, &git, &payload)?;
    let (repo, github) = ci_repo(&provider, env, &git, &token, &payload)?;

    let images = transform_images(&images)?;
    let mut tags = transform_tags(&tags)?;
    let flavor = transform_flavor(&flavor)?;
    let mut meta = Meta {
        ctx: &context,
        flavor,
        env,
        warnings: Vec::new(),
    };
    let version = meta.version(&mut tags, &repo)?;
    if version.main.as_deref().is_none_or(str::is_empty) {
        meta.warnings
            .push("No Docker image version has been generated. Check tags input.".into());
    }
    let out_tags = meta.tags(&images, &version);
    if out_tags.is_empty() {
        meta.warnings
            .push("No Docker tag has been generated. Check tags input.".into());
    }
    let out_labels = oci_labels(&labels, &repo, &context, &version)?;
    Ok(MetaOut {
        tags: out_tags,
        labels: out_labels,
        github,
        warnings: meta.warnings,
    })
}

struct Image {
    name: String,
    enable: bool,
}

/// image.js `Transform`: one entry of bare names is the old comma-separated
/// format; otherwise `name=…,enable=…` per entry.
fn transform_images(inputs: &[String]) -> Result<Vec<Image>> {
    let split =
        |field: &str| -> Vec<String> { field.split('=').map(|p| js_trim(p).to_string()).collect() };
    if let [only] = inputs {
        let mut images = Vec::new();
        let mut new_format = false;
        for field in first_record(only, "images")? {
            let parts = split(&field);
            if parts.len() == 1 {
                images.push(Image {
                    name: parts[0].clone(),
                    enable: true,
                });
            } else {
                new_format = true;
                break;
            }
        }
        if !new_format {
            return Ok(images);
        }
    }
    let mut images = Vec::new();
    for input in inputs {
        let mut image = Image {
            name: String::new(),
            enable: true,
        };
        for field in first_record(input, "images")? {
            let parts = split(&field);
            if parts.len() == 1 {
                image.name = parts[0].clone();
                continue;
            }
            let value = &parts[1];
            match parts[0].to_lowercase().as_str() {
                "name" => image.name = value.clone(),
                "enable" => {
                    if value != "true" && value != "false" {
                        bail!("Invalid enable attribute value: {input}");
                    }
                    image.enable = value == "true";
                }
                _ => bail!("Unknown image attribute: {input}"),
            }
        }
        if image.name.is_empty() {
            bail!("Image name attribute empty: {input}");
        }
        images.push(image);
    }
    Ok(images)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagType {
    Schedule,
    Semver,
    Pep440,
    Match,
    Edge,
    Ref,
    Raw,
    Sha,
}

impl TagType {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "schedule" => TagType::Schedule,
            "semver" => TagType::Semver,
            "pep440" => TagType::Pep440,
            "match" => TagType::Match,
            "edge" => TagType::Edge,
            "ref" => TagType::Ref,
            "raw" => TagType::Raw,
            "sha" => TagType::Sha,
            _ => return None,
        })
    }
    fn default_priority(self) -> &'static str {
        match self {
            TagType::Schedule => "1000",
            TagType::Semver | TagType::Pep440 => "900",
            TagType::Match => "800",
            TagType::Edge => "700",
            TagType::Ref => "600",
            TagType::Raw => "200",
            TagType::Sha => "100",
        }
    }
}

struct Tag {
    ty: TagType,
    attrs: BTreeMap<String, String>,
}

impl Tag {
    fn attr(&self, key: &str) -> &str {
        self.attrs.get(key).map_or("", String::as_str)
    }
    fn has(&self, key: &str) -> bool {
        self.attrs.contains_key(key)
    }
}

/// tag.js `Parse`. A field splits at its first `=` unless that `=` starts
/// the field (`/(?<=^[^=]+?)=/`); a field without one is the value.
fn parse_tag(s: &str) -> Result<Tag> {
    let mut ty = None;
    let mut attrs = BTreeMap::new();
    for field in first_record(s, "tags")? {
        match field.find('=').filter(|i| *i > 0) {
            None => {
                attrs.insert("value".to_string(), js_trim(&field).to_string());
            }
            Some(i) => {
                let key = js_trim(&field[..i]).to_lowercase();
                let value = js_trim(&field[i + 1..]).to_string();
                if key == "type" {
                    ty = Some(
                        TagType::parse(&value)
                            .ok_or_else(|| eyre!("Unknown tag type attribute: {value}"))?,
                    );
                } else {
                    attrs.insert(key, value);
                }
            }
        }
    }
    let ty = ty.unwrap_or(TagType::Raw);
    let mut tag = Tag { ty, attrs };
    let default = |tag: &mut Tag, k: &str, v: &str| {
        tag.attrs
            .entry(k.to_string())
            .or_insert_with(|| v.to_string());
    };
    match ty {
        TagType::Schedule => default(&mut tag, "pattern", "nightly"),
        TagType::Semver | TagType::Pep440 => {
            if !tag.has("pattern") {
                bail!("Missing pattern attribute for {s}");
            }
            default(&mut tag, "value", "");
        }
        TagType::Match => {
            if !tag.has("pattern") {
                bail!("Missing pattern attribute for {s}");
            }
            default(&mut tag, "group", "0");
            if js_number(tag.attr("group")).is_nan() {
                bail!("Invalid match group for {s}");
            }
            default(&mut tag, "value", "");
        }
        TagType::Edge => default(&mut tag, "branch", ""),
        TagType::Ref => {
            if !tag.has("event") {
                bail!("Missing event attribute for {s}");
            }
            if !matches!(tag.attr("event"), "branch" | "tag" | "pr") {
                bail!("Invalid event for {s}");
            }
            if tag.attr("event") == "pr" {
                default(&mut tag, "prefix", "pr-");
            }
        }
        TagType::Raw => {
            if !tag.has("value") {
                bail!("Missing value attribute for {s}");
            }
        }
        TagType::Sha => {
            default(&mut tag, "prefix", "sha-");
            default(&mut tag, "format", "short");
            if !matches!(tag.attr("format"), "short" | "long") {
                bail!("Invalid format for {s}");
            }
        }
    }
    default(&mut tag, "enable", "true");
    default(&mut tag, "priority", ty.default_priority());
    Ok(tag)
}

/// tag.js `Transform`: the four default tags when none are given, stably
/// sorted by numeric priority, highest first. A non-numeric priority makes
/// the JavaScript comparator inconsistent (the result depends on V8's sort
/// internals), so it is refused.
fn transform_tags(inputs: &[String]) -> Result<Vec<Tag>> {
    let defaults = [
        "type=schedule",
        "type=ref,event=branch",
        "type=ref,event=tag",
        "type=ref,event=pr",
    ]
    .map(String::from);
    let inputs = if inputs.is_empty() {
        &defaults[..]
    } else {
        inputs
    };
    let mut keyed = Vec::new();
    for input in inputs {
        let tag = parse_tag(input)?;
        let priority = js_number(tag.attr("priority"));
        if priority.is_nan() {
            bail!(
                "tag `{input}`: priority `{}` is not a number (the executor's sort is \
                 undefined then)",
                tag.attr("priority")
            );
        }
        keyed.push((priority, tag));
    }
    keyed.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    Ok(keyed.into_iter().map(|(_, t)| t).collect())
}

struct Flavor {
    latest: String,
    prefix: String,
    prefix_latest: bool,
    suffix: String,
    suffix_latest: bool,
}

/// flavor.js `Transform`.
fn transform_flavor(inputs: &[String]) -> Result<Flavor> {
    let mut f = Flavor {
        latest: "auto".into(),
        prefix: String::new(),
        prefix_latest: false,
        suffix: String::new(),
        suffix_latest: false,
    };
    for input in inputs {
        let mut on_latest_for = "";
        for field in first_record(input, "flavor")? {
            let parts: Vec<&str> = field.split('=').map(js_trim).collect();
            if parts.len() == 1 {
                bail!("Invalid flavor entry: {input}");
            }
            let value = parts[1];
            match parts[0].to_lowercase().as_str() {
                "latest" => {
                    if !matches!(value, "auto" | "true" | "false") {
                        bail!("Invalid latest flavor entry: {input}");
                    }
                    f.latest = value.into();
                }
                "prefix" => {
                    f.prefix = value.into();
                    on_latest_for = "prefix";
                }
                "suffix" => {
                    f.suffix = value.into();
                    on_latest_for = "suffix";
                }
                "onlatest" => {
                    if !matches!(value, "true" | "false") {
                        bail!("Invalid value for onlatest attribute: {value}");
                    }
                    match on_latest_for {
                        "prefix" => f.prefix_latest = value == "true",
                        "suffix" => f.suffix_latest = value == "true",
                        _ => {}
                    }
                }
                _ => bail!("Unknown flavor entry: {input}"),
            }
        }
    }
    Ok(f)
}

#[derive(Default)]
struct Version {
    main: Option<String>,
    partial: Vec<String>,
    latest: Option<bool>,
}

const DATE_UNSUPPORTED: &str = "the handlebars `date` helper is not ported to butler (it \
                                formats the build time with moment-timezone); run this target \
                                through nx";

/// meta.js `Meta`.
struct Meta<'a> {
    ctx: &'a CiContext,
    flavor: Flavor,
    env: &'a Env,
    warnings: Vec<String>,
}

impl Meta<'_> {
    /// `flavor.latest == 'auto' ? auto : flavor.latest == 'true'`.
    fn latest(&self, auto: bool) -> bool {
        match self.flavor.latest.as_str() {
            "auto" => auto,
            l => l == "true",
        }
    }

    /// `setGlobalExp`: the template with the global expressions.
    fn global_exp(&self, val: &str) -> Result<String> {
        let ctx = self.ctx;
        hb_render(val, &|name| {
            Ok(Some(match name {
                "branch" if ctx.ref_starts("refs/heads/") => ctx.ref_without("refs/heads/"),
                "tag" if ctx.ref_starts("refs/tags/") => ctx.ref_without("refs/tags/"),
                "branch" | "tag" => String::new(),
                "sha" => match &ctx.sha {
                    Some(sha) => short_sha(sha, self.env)?,
                    None => bail!("{{{{sha}}}}: the CI context has no commit sha"),
                },
                "base_ref" => self.base_ref()?,
                "is_default_branch" => self.is_default_branch()?.into(),
                "date" => bail!(DATE_UNSUPPORTED),
                _ => return Ok(None),
            }))
        })
    }

    fn base_ref(&self) -> Result<String> {
        let payload = &self.ctx.payload;
        if self.ctx.ref_starts("refs/tags/")
            && let Some(base) = payload.get("base_ref").filter(|v| !v.is_null())
        {
            let base = base
                .as_str()
                .ok_or_else(|| eyre!("event payload base_ref is not a string"))?;
            return Ok(base.strip_prefix("refs/heads/").unwrap_or(base).to_string());
        }
        if self.ctx.ref_starts("refs/pull/")
            && let Some(base) = payload
                .get("pull_request")
                .and_then(|p| p.get("base"))
                .and_then(|b| b.get("ref"))
                .filter(|v| !v.is_null())
        {
            return Ok(js_string(base));
        }
        Ok(String::new())
    }

    fn is_default_branch(&self) -> Result<&'static str> {
        let Some(r) = self.ctx.git_ref.as_deref() else {
            bail!("{{{{is_default_branch}}}}: the CI context has no ref");
        };
        let branch = r.strip_prefix("refs/heads/").unwrap_or(r);
        if branch.is_empty() {
            return Ok("false");
        }
        let default = self
            .ctx
            .payload
            .get("repository")
            .and_then(|r| r.get("default_branch"))
            .filter(|v| !v.is_null());
        if default.is_some_and(|d| js_string(d) == branch) {
            return Ok("true");
        }
        if ["create", "discussion", "issues", "schedule"]
            .iter()
            .any(|e| self.ctx.event_matches(e))
        {
            return Ok("true");
        }
        Ok("false")
    }

    /// `setValue`: the tag's (else the flavor's) prefix and suffix.
    fn set_value(&self, val: &str, tag: &Tag) -> Result<String> {
        let mut val = val.to_string();
        if tag.has("prefix") {
            val = format!("{}{val}", self.global_exp(tag.attr("prefix"))?);
        } else if !self.flavor.prefix.is_empty() {
            val = format!("{}{val}", self.global_exp(&self.flavor.prefix)?);
        }
        if tag.has("suffix") {
            val = format!("{val}{}", self.global_exp(tag.attr("suffix"))?);
        } else if !self.flavor.suffix.is_empty() {
            val = format!("{val}{}", self.global_exp(&self.flavor.suffix)?);
        }
        Ok(val)
    }

    /// `getVersion`.
    fn version(&mut self, tags: &mut [Tag], repo: &Repo) -> Result<Version> {
        let mut v = Version::default();
        for tag in tags.iter_mut() {
            let enabled = self.global_exp(tag.attr("enable"))?;
            if enabled != "true" && enabled != "false" {
                bail!("Invalid value for enable attribute: {enabled}");
            }
            if enabled != "true" {
                continue;
            }
            let next = match tag.ty {
                TagType::Schedule => self.proc_schedule(tag)?,
                TagType::Semver => self.proc_semver(tag)?,
                TagType::Pep440 => self.proc_unported(tag, "pep440", "@renovatebot/pep440")?,
                TagType::Match => self.proc_unported(tag, "match", "a JavaScript RegExp")?,
                TagType::Ref => match tag.attr("event") {
                    "branch" if self.ctx.ref_starts("refs/heads/") => Some((
                        self.set_value(&self.ctx.ref_without("refs/heads/"), tag)?,
                        self.latest(false),
                    )),
                    "tag" if self.ctx.ref_starts("refs/tags/") => Some((
                        self.set_value(&self.ctx.ref_without("refs/tags/"), tag)?,
                        self.latest(true),
                    )),
                    "pr" if self.ctx.ref_starts("refs/pull/") => {
                        let pr = self.ctx.ref_without("refs/pull/");
                        let pr = pr.strip_suffix("/merge").unwrap_or(&pr);
                        Some((self.set_value(pr, tag)?, self.latest(false)))
                    }
                    _ => None,
                },
                TagType::Edge => self.proc_edge(tag, repo)?,
                TagType::Raw => Some((
                    self.set_value(&self.global_exp(tag.attr("value"))?, tag)?,
                    self.latest(false),
                )),
                TagType::Sha => match self.ctx.sha.as_deref().filter(|s| !s.is_empty()) {
                    None => None,
                    Some(sha) => {
                        let val = if tag.attr("format") == "short" {
                            short_sha(sha, self.env)?
                        } else {
                            sha.to_string()
                        };
                        Some((self.set_value(&val, tag)?, self.latest(false)))
                    }
                },
            };
            if let Some((val, latest)) = next {
                set_version(&mut v, &val, latest);
            }
        }
        let mut seen = Vec::new();
        v.partial.retain(|p| {
            let first = !seen.contains(p);
            seen.push(p.clone());
            first
        });
        v.latest.get_or_insert(false);
        Ok(v)
    }

    fn proc_schedule(&self, tag: &Tag) -> Result<Option<(String, bool)>> {
        if !self.ctx.event_matches("schedule") {
            return Ok(None);
        }
        // The pattern's only context is the `date` helper.
        let raw = hb_render(tag.attr("pattern"), &|name| {
            if name == "date" {
                bail!(DATE_UNSUPPORTED);
            }
            Ok(None)
        })?;
        Ok(Some((self.set_value(&raw, tag)?, self.latest(false))))
    }

    fn proc_semver(&mut self, tag: &Tag) -> Result<Option<(String, bool)>> {
        let value = tag.attr("value");
        if !self.ctx.ref_starts("refs/tags/") && value.is_empty() {
            return Ok(None);
        }
        let vraw = if value.is_empty() {
            self.ctx.ref_without("refs/tags/").replace('/', "-")
        } else {
            self.global_exp(value)?
        };
        let Some(sver) = SemVer::parse(&vraw) else {
            self.warnings.push(format!(
                "{vraw} is not a valid semver. More info: https://semver.org/"
            ));
            return Ok(None);
        };
        let pattern = tag.attr("pattern");
        let (val, latest) = if !sver.prerelease.is_empty() {
            let pattern = if is_raw_statement(pattern)? {
                pattern
            } else {
                "{{version}}"
            };
            (self.set_value(&sver.render(pattern)?, tag)?, false)
        } else {
            (self.set_value(&sver.render(pattern)?, tag)?, true)
        };
        Ok(Some((val, self.latest(latest))))
    }

    /// Types whose value needs a library that is not ported: refused where
    /// the executor would start using it.
    fn proc_unported(&self, tag: &Tag, ty: &str, needs: &str) -> Result<Option<(String, bool)>> {
        if !self.ctx.ref_starts("refs/tags/") && tag.attr("value").is_empty() {
            return Ok(None);
        }
        bail!(
            "tag type `{ty}` is not ported to butler (it needs {needs}); run this target \
             through nx"
        )
    }

    fn proc_edge(&self, tag: &mut Tag, repo: &Repo) -> Result<Option<(String, bool)>> {
        if !self.ctx.ref_starts("refs/heads/") {
            return Ok(None);
        }
        let current = self.ctx.ref_without("refs/heads/");
        let branch = if tag.attr("branch").is_empty() {
            match repo {
                Repo::Known(k) => k.default_branch.clone(),
                Repo::GitHub => bail!(
                    "type=edge without branch= compares with the repository's default branch, \
                     which butler only learns from the GitHub API when the step runs; set \
                     branch=<name>"
                ),
            }
        } else {
            Some(tag.attr("branch").to_string())
        };
        if let Some(b) = &branch {
            tag.attrs.insert("branch".into(), b.clone());
        }
        if branch.as_deref() != Some(current.as_str()) {
            return Ok(None);
        }
        Ok(Some((self.set_value("edge", tag)?, self.latest(false))))
    }

    /// `getTags`.
    fn tags(&self, images: &[Image], v: &Version) -> Vec<String> {
        let Some(main) = v.main.as_deref().filter(|m| !m.is_empty()) else {
            return Vec::new();
        };
        let generate = |image: &str| {
            let prefix = if image.is_empty() {
                String::new()
            } else {
                format!("{image}:")
            };
            let mut out = vec![format!("{prefix}{main}")];
            out.extend(v.partial.iter().map(|p| format!("{prefix}{p}")));
            if v.latest == Some(true) {
                let f = &self.flavor;
                let latest = format!(
                    "{}latest{}",
                    if f.prefix_latest {
                        f.prefix.as_str()
                    } else {
                        ""
                    },
                    if f.suffix_latest {
                        f.suffix.as_str()
                    } else {
                        ""
                    }
                );
                out.push(format!("{prefix}{}", sanitize_tag(&latest)));
            }
            out
        };
        let names: Vec<String> = images
            .iter()
            .filter(|i| i.enable)
            .map(|i| interpolate(&i.name, self.env).to_lowercase())
            .collect();
        if names.is_empty() {
            return generate("");
        }
        names.iter().flat_map(|n| generate(n)).collect()
    }
}

/// `Meta.setVersion`.
fn set_version(v: &mut Version, val: &str, latest: bool) {
    if val.is_empty() {
        return;
    }
    let val = sanitize_tag(val);
    match &v.main {
        None => v.main = Some(val),
        Some(main) if *main != val => v.partial.push(val),
        Some(_) => {}
    }
    v.latest.get_or_insert(latest);
}

/// `/[^a-zA-Z0-9._-]+/g` → `-`.
fn sanitize_tag(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len());
    let mut in_run = false;
    for c in tag.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    out
}

/// `Meta.shortSha`: `NX_CONTAINER_SHORT_SHA_LENGTH` (default 7) characters.
fn short_sha(sha: &str, env: &Env) -> Result<String> {
    let mut len = 7.0;
    if let Some(v) = env_truthy(env, "NX_CONTAINER_SHORT_SHA_LENGTH") {
        len = js_number(v);
        if len.is_nan() {
            bail!("NX_CONTAINER_SHORT_SHA_LENGTH is not a valid number: {v}");
        }
    }
    let chars = sha.chars().count();
    if len >= chars as f64 {
        return Ok(sha.to_string());
    }
    // `substring(0, len)`: truncated toward zero, clamped at 0.
    let take = if len > 0.0 { len.trunc() as usize } else { 0 };
    Ok(sha.chars().take(take).collect())
}

/// meta.js `isRawStatement`: the pattern is exactly `{{raw}}`.
fn is_raw_statement(pattern: &str) -> Result<bool> {
    Ok(matches!(hb_parse(pattern)?.as_slice(), [Hb::Var { name, .. }] if name == "raw"))
}

/// `getOCIAnnotationsWithCustoms(labels)`: the OCI labels plus custom ones,
/// keyed at the first `=` (later keys win, entries without `=` dropped),
/// sorted by `localeCompare` of the key.
fn oci_labels(
    extra: &[String],
    repo: &Repo,
    ctx: &CiContext,
    v: &Version,
) -> Result<Vec<Vec<Piece>>> {
    let repo_piece = |f: RepoField| match repo {
        Repo::GitHub => Piece::Repo(f),
        Repo::Known(k) => Piece::Lit(match f {
            RepoField::Name => k.name.clone().unwrap_or_default(),
            RepoField::Description => k.description.clone().unwrap_or_default(),
            RepoField::HtmlUrl => k.html_url.clone().unwrap_or_default(),
            RepoField::License => String::new(),
        }),
    };
    let lit = |s: &str| Piece::Lit(s.to_string());
    let mut entries: Vec<(String, Vec<Piece>)> = vec![
        (
            "org.opencontainers.image.title".into(),
            vec![repo_piece(RepoField::Name)],
        ),
        (
            "org.opencontainers.image.description".into(),
            vec![repo_piece(RepoField::Description)],
        ),
        (
            "org.opencontainers.image.url".into(),
            vec![repo_piece(RepoField::HtmlUrl)],
        ),
        (
            "org.opencontainers.image.source".into(),
            vec![repo_piece(RepoField::HtmlUrl)],
        ),
        (
            "org.opencontainers.image.version".into(),
            vec![lit(v.main.as_deref().unwrap_or_default())],
        ),
        ("org.opencontainers.image.created".into(), vec![Piece::Now]),
        (
            "org.opencontainers.image.revision".into(),
            vec![lit(ctx.sha.as_deref().unwrap_or_default())],
        ),
        (
            "org.opencontainers.image.licenses".into(),
            vec![repo_piece(RepoField::License)],
        ),
    ];
    for label in extra {
        let Some((key, value)) = label.split_once('=') else {
            continue;
        };
        match entries.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = vec![lit(value)],
            None => entries.push((key.to_string(), vec![lit(value)])),
        }
    }
    if let Some((k, _)) = entries
        .iter()
        .find(|(k, _)| !k.chars().all(|c| (' '..='~').contains(&c)))
    {
        bail!(
            "label key `{k}` is outside printable ASCII: its ICU collation order \
             (localeCompare) is not ported"
        );
    }
    entries.sort_by(|a, b| locale_compare(&a.0, &b.0));
    Ok(entries
        .into_iter()
        .map(|(k, mut v)| {
            v.insert(0, Piece::Lit(format!("{k}=")));
            v
        })
        .collect())
}

/// `a.localeCompare(b)` (ICU root collation, what node's default `en-US`
/// uses for ASCII) for printable-ASCII strings: primary weights first —
/// punctuation in ICU's order, then digits, then letters case-blind — then
/// lowercase before uppercase.
fn locale_compare(a: &str, b: &str) -> Ordering {
    const ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789abcdefghijklmnopqrstuvwxyz";
    let primary = |s: &str| -> Vec<usize> {
        s.chars()
            .map(|c| ORDER.find(c.to_ascii_lowercase()).unwrap_or(usize::MAX))
            .collect()
    };
    let tertiary = |s: &str| -> Vec<bool> { s.chars().map(|c| c.is_ascii_uppercase()).collect() };
    primary(a)
        .cmp(&primary(b))
        .then_with(|| tertiary(a).cmp(&tertiary(b)))
}

/// node-semver 7 `SemVer` for a strictly valid version (`semver.valid`), as
/// container-metadata hands it to handlebars.
struct SemVer {
    raw: String,
    major: u64,
    minor: u64,
    patch: u64,
    /// Numeric identifiers are numbers in node-semver; both print the same.
    prerelease: Vec<String>,
    build: Vec<String>,
}

impl SemVer {
    const MAX_SAFE: u64 = 9_007_199_254_740_991;

    /// `new SemVer(v)` (strict `FULL` regex): `v?MAJOR.MINOR.PATCH`, optional
    /// `-prerelease`, optional `+build`, after trimming; `None` when invalid.
    fn parse(v: &str) -> Option<SemVer> {
        if v.encode_utf16().count() > 256 {
            return None;
        }
        let s = js_trim(v);
        let s = s.strip_prefix('v').unwrap_or(s);
        let end = s.find(['-', '+']).unwrap_or(s.len());
        let numeric = |id: &str| -> Option<u64> {
            let ok = id == "0"
                || (id.starts_with(|c: char| ('1'..='9').contains(&c))
                    && id.chars().all(|c| c.is_ascii_digit()));
            if !ok || id.len() > 16 {
                return None;
            }
            id.parse().ok().filter(|n| *n <= Self::MAX_SAFE)
        };
        let main: Vec<&str> = s[..end].split('.').collect();
        let [major, minor, patch] = main.as_slice() else {
            return None;
        };
        let (major, minor, patch) = (numeric(major)?, numeric(minor)?, numeric(patch)?);
        let mut rest = &s[end..];
        let mut prerelease = Vec::new();
        if let Some(r) = rest.strip_prefix('-') {
            let pe = r.find('+').unwrap_or(r.len());
            for id in r[..pe].split('.') {
                let chars_ok =
                    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
                let all_digits = id.chars().all(|c| c.is_ascii_digit());
                if !chars_ok || (all_digits && id != "0" && id.starts_with('0')) {
                    return None;
                }
                // `/^[0-9]+$/` identifiers below MAX_SAFE_INTEGER become numbers.
                let as_number = all_digits
                    .then(|| id.parse::<u64>().ok())
                    .flatten()
                    .filter(|n| *n < Self::MAX_SAFE);
                prerelease.push(as_number.map_or_else(|| id.to_string(), |n| n.to_string()));
            }
            rest = &r[pe..];
        }
        let mut build = Vec::new();
        if let Some(r) = rest.strip_prefix('+') {
            for id in r.split('.') {
                if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    return None;
                }
                build.push(id.to_string());
            }
            rest = "";
        }
        rest.is_empty().then(|| SemVer {
            raw: v.to_string(),
            major,
            minor,
            patch,
            prerelease,
            build,
        })
    }

    fn version(&self) -> String {
        let mut v = format!("{}.{}.{}", self.major, self.minor, self.patch);
        if !self.prerelease.is_empty() {
            v.push('-');
            v.push_str(&self.prerelease.join("."));
        }
        v
    }

    /// The pattern over the parsed (loose) `SemVer`'s own properties.
    fn render(&self, pattern: &str) -> Result<String> {
        hb_render(pattern, &|name| {
            Ok(Some(match name {
                "raw" => self.raw.clone(),
                "version" => self.version(),
                "major" => self.major.to_string(),
                "minor" => self.minor.to_string(),
                "patch" => self.patch.to_string(),
                "prerelease" => self.prerelease.join(","),
                "build" => self.build.join(","),
                "loose" => "true".into(),
                "includePrerelease" => "false".into(),
                "options" => "[object Object]".into(),
                _ => return Ok(None),
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

/// core `exec` (tinyexec) runs processes with `node_modules/.bin` of the cwd
/// and every ancestor prepended to PATH. (It also adds the JS runtime's own
/// directory, which butler does not have.)
fn exec_env(cwd: &Path, env: &Env) -> Env {
    let mut out = env.clone();
    let (key, value) = env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("path"))
        .filter(|(_, v)| !v.is_empty())
        .map_or(("PATH".to_string(), String::new()), |(k, v)| {
            (k.clone(), v.clone())
        });
    let mut dirs = Vec::new();
    let mut dir = cwd.to_path_buf();
    loop {
        dirs.push(
            dir.join("node_modules")
                .join(".bin")
                .to_string_lossy()
                .into_owned(),
        );
        if !dir.pop() {
            break;
        }
    }
    dirs.push(value);
    out.insert(key, dirs.join(":"));
    out
}

struct Captured {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn command(program: &str, args: &[String], cwd: &Path, env: &Env) -> Command {
    let mut c = Command::new(program);
    c.args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null());
    c
}

fn capture(program: &str, args: &[String], cwd: &Path, env: &Env) -> std::io::Result<Captured> {
    let out = command(program, args, cwd, env).output()?;
    Ok(Captured {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code(),
    })
}

/// The build runs with inherited stdio. Here stdout and stderr share one
/// open file description (`sink`, created fresh), so the captured output
/// keeps their interleaving; it is read back once the process exits.
fn merged(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &Env,
    sink: &Path,
) -> std::io::Result<(String, Option<i32>)> {
    let file = fs::File::create_new(sink)?;
    let status = command(program, args, cwd, env)
        .stdout(file.try_clone()?)
        .stderr(file)
        .status()?;
    let mut buf = Vec::new();
    fs::File::open(sink)?.read_to_end(&mut buf)?;
    Ok((String::from_utf8_lossy(&buf).into_owned(), status.code()))
}

/// buildx.js `getCommand`.
fn buildx_cmd(args: &[&str], standalone: bool) -> (String, Vec<String>) {
    let args = args.iter().map(|a| a.to_string());
    if standalone {
        ("buildx".into(), args.collect())
    } else {
        (
            "docker".into(),
            std::iter::once("buildx".to_string()).chain(args).collect(),
        )
    }
}

/// docker.js / buildx.js `isAvailable`: exit 0, spawn failures count as no.
fn available(program: &str, args: &[String], cwd: &Path, env: &Env) -> bool {
    capture(program, args, cwd, env).is_ok_and(|r| r.code == Some(0))
}

/// A non-silent `exec(..., { throwOnError: false })`: output printed, exit
/// status ignored, spawn failures fatal.
fn log_exec(
    log: &mut String,
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &Env,
) -> Result<Captured> {
    let r = capture(program, args, cwd, env).map_err(|e| eyre!("spawn {program}: {e}"))?;
    log.push_str(&r.stdout);
    log.push_str(&r.stderr);
    Ok(r)
}

/// buildx.js `parseVersion`: `/\sv?([0-9a-f]{7}|[0-9.]+)/`.
fn parse_buildx_version(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if !is_js_space(*c) {
            continue;
        }
        let starts = if chars.get(i + 1) == Some(&'v') {
            vec![i + 2, i + 1]
        } else {
            vec![i + 1]
        };
        for st in starts {
            let rest = chars.get(st..).unwrap_or_default();
            if rest.len() >= 7 && rest[..7].iter().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
                return Some(rest[..7].iter().collect());
            }
            let run: String = rest
                .iter()
                .take_while(|c| c.is_ascii_digit() || **c == '.')
                .collect();
            if !run.is_empty() {
                return Some(run);
            }
        }
    }
    None
}

/// buildx.js `satisfies(version, '>=a.b.c')`: a valid semver at least that,
/// or a 7-hex-digit commit (development builds pass every gate).
fn buildx_satisfies(version: &str, min: (u64, u64, u64)) -> bool {
    if version.len() == 7 && version.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        return true;
    }
    SemVer::parse(version)
        .is_some_and(|v| v.prerelease.is_empty() && (v.major, v.minor, v.patch) >= min)
}

/// `last line` of stderr as `stderr.match(/(.*)\s*$/)[0].trim()` picks it.
fn last_line(stderr: &str) -> String {
    let mut starts: Vec<usize> = stderr.char_indices().map(|(i, _)| i).collect();
    starts.push(stderr.len());
    for p in starts {
        let rest = &stderr[p..];
        let k = rest.find(is_line_terminator).unwrap_or(rest.len());
        if rest[k..].chars().all(is_js_space) {
            return js_trim(rest).to_string();
        }
    }
    String::new()
}

fn random_u64() -> u64 {
    let mut h = RandomState::new().build_hasher();
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.write_u32(std::process::id());
    h.finish()
}

/// node `os.tmpdir()` + `fs.mkdtempSync(prefix)`.
fn make_tmp_dir(env: &Env) -> Result<PathBuf> {
    let base = ["TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|k| env_truthy(env, k))
        .unwrap_or("/tmp");
    let base = if base.len() > 1 {
        base.strip_suffix('/').unwrap_or(base)
    } else {
        base
    };
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    for _ in 0..100 {
        let mut r = random_u64();
        let suffix: String = (0..6)
            .map(|_| {
                let c = ALPHABET[(r % ALPHABET.len() as u64) as usize] as char;
                r /= ALPHABET.len() as u64;
                c
            })
            .collect();
        let dir = PathBuf::from(format!("{base}/docker-build-push-{suffix}"));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => bail!("mkdtemp {}: {e}", dir.display()),
        }
    }
    bail!("mkdtemp {base}/docker-build-push-XXXXXX: no free name")
}

/// `new Date().toISOString()`.
fn iso_now() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis()) as i64;
    let (days, ms_of_day) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        ms_of_day / 3_600_000,
        ms_of_day / 60_000 % 60,
        ms_of_day / 1000 % 60,
        ms_of_day % 1000
    )
}

/// ci-context `github.repo(token)`: `GET /repos/{owner}/{repo}` as octokit
/// sends it (curl, with the token passed on stdin, never on argv).
fn fetch_github_repo(g: &GithubRepo, cwd: &Path, env: &Env) -> Result<RepoValues> {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                b => format!("%{b:02X}"),
            })
            .collect()
    };
    let url = format!("{}/repos/{}/{}", g.api_url, enc(&g.owner), enc(&g.repo));
    // @octokit/auth-token: JWTs (three dot-separated parts) are bearer tokens.
    let scheme = if g.token.split('.').count() == 3 {
        "bearer"
    } else {
        "token"
    };
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let config = format!(
        "url = {}\nheader = {}\nheader = {}\nheader = {}\n",
        quote(&url),
        quote("Accept: application/vnd.github.v3+json"),
        quote("User-Agent: butler (@nx-tools/ci-context port)"),
        quote(&format!("Authorization: {scheme} {}", g.token)),
    );
    let args: Vec<String> = [
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--config",
        "-",
    ]
    .map(String::from)
    .to_vec();
    let mut child = command("curl", &args, cwd, env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| eyre!("GET {url}: spawn curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(config.as_bytes())
            .map_err(|e| eyre!("GET {url}: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| eyre!("GET {url}: {e}"))?;
    if !out.status.success() {
        bail!(
            "GET {url}: {}",
            String::from_utf8_lossy(&out.stderr).trim_end()
        );
    }
    let data: Json = serde_json::from_slice(&out.stdout).map_err(|e| eyre!("GET {url}: {e}"))?;
    let field = |v: Option<&Json>| match v {
        Some(v) if is_truthy(v) => js_string(v),
        _ => String::new(),
    };
    Ok(RepoValues {
        name: field(data.get("name")),
        description: field(data.get("description")),
        html_url: field(data.get("html_url")),
        license: field(data.get("license").and_then(|l| l.get("spdx_id"))),
    })
}

/// `context.setOutput`.
fn set_output(root: &Path, project: &str, name: &str, value: &str) -> Result<()> {
    if project.is_empty() {
        return Ok(());
    }
    let dir = root.join("node_modules/.cache/nx-container").join(project);
    fs::create_dir_all(&dir).map_err(|e| eyre!("mkdir {}: {e}", dir.display()))?;
    fs::write(dir.join(name), value).map_err(|e| eyre!("write {}/{name}: {e}", dir.display()))
}

fn read_lossy(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|e| eyre!("read {}: {e}", path.display()))?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn run(spec: &Spec, ctx: &NativeCtx<'_>) -> Result<StepOutput> {
    let mut log = String::new();
    let tmp = make_tmp_dir(ctx.env)?;
    let mut result = execute(spec, ctx, &tmp, &mut log);
    if tmp.exists() {
        let _ = writeln!(log, "Removing temp folder {}", tmp.display());
        if let Err(e) = fs::remove_dir_all(&tmp) {
            result = result.and(Err(eyre!("rm {}: {e}", tmp.display())));
        }
    }
    let success = match result {
        Ok(()) => true,
        Err(e) => {
            let _ = writeln!(log, "Error: {e:#}");
            false
        }
    };
    Ok(StepOutput {
        success,
        output: log,
    })
}

/// executor.js `runExecutor`, from `engine.initialize` on.
fn execute(spec: &Spec, ctx: &NativeCtx<'_>, tmp: &Path, log: &mut String) -> Result<()> {
    let root = ctx.workspace_root;
    let penv = exec_env(root, ctx.env);

    // Docker.initialize: standalone when the docker CLI does not answer.
    let standalone = !available("docker", &[], root, &penv);
    if !spec.quiet {
        if standalone {
            log.push_str("Docker info skipped in standalone mode\n");
        } else {
            log_exec(log, "docker", &["version".into()], root, &penv)?;
            log_exec(log, "docker", &["info".into()], root, &penv)?;
        }
    }
    let (program, args) = buildx_cmd(&[], standalone);
    if !available(&program, &args, root, &penv) {
        bail!(
            "Docker buildx is required. See https://github.com/gperdomor/oss to set up \
             nx-container executor with buildx."
        );
    }
    // `buildx.getVersion()` is called without `standalone`, so it always asks
    // `docker buildx version`.
    let (program, args) = buildx_cmd(&["version"], false);
    let v = capture(&program, &args, root, &penv).map_err(|e| eyre!("spawn {program}: {e}"))?;
    if !v.stderr.is_empty() && v.code != Some(0) {
        bail!("{}", js_trim(&v.stderr));
    }
    let version = parse_buildx_version(js_trim(&v.stdout))
        .ok_or_else(|| eyre!("Cannot parse buildx version"))?;
    if !spec.quiet {
        let (program, args) = buildx_cmd(&["version"], standalone);
        log_exec(log, &program, &args, root, &penv)?;
    }
    let builder = if spec.create_builder {
        let name = if spec.builder.is_empty() {
            format!("{}-{:06x}", spec.project, random_u64() & 0xff_ffff)
        } else {
            spec.builder.clone()
        };
        let _ = writeln!(log, "Creating builder {name}");
        let (program, args) = buildx_cmd(&["create", &format!("--name={name}")], standalone);
        let r = log_exec(log, &program, &args, root, &penv)?;
        if !r.stderr.is_empty() && r.code != Some(0) {
            bail!("buildx failed with: {}", last_line(&r.stderr));
        }
        name
    } else {
        spec.builder.clone()
    };

    // getMetadata.
    for w in &spec.meta_warnings {
        let _ = writeln!(log, "Warning: {w}");
    }
    let repo = match &spec.github {
        Some(g) => fetch_github_repo(g, root, ctx.env)?,
        None => RepoValues::default(),
    };
    let now = iso_now();

    // getArgs: secrets become files in the temp dir.
    let mut written = vec![false; spec.secrets.len()];
    for (i, s) in spec.secrets.iter().enumerate() {
        let (content, file) = match s {
            SecretEntry::Invalid(w) => {
                let _ = writeln!(log, "Warning: {w}");
                continue;
            }
            SecretEntry::Value { value, file } => (interpolate(value, ctx.env), file),
            SecretEntry::File { path, file } => {
                let path = interpolate(path, ctx.env);
                match read_lossy(&root.join(&path))? {
                    Some(c) => (c, file),
                    None => {
                        let _ = writeln!(log, "Warning: secret file {path} not found");
                        continue;
                    }
                }
            }
        };
        fs::write(tmp.join(file), content).map_err(|e| eyre!("write secret {file}: {e}"))?;
        written[i] = true;
    }

    let tmp_str = tmp.to_string_lossy();
    let r = Render::Run {
        tmp: &tmp_str,
        now: &now,
        repo: &repo,
        builder: &builder,
    };
    let passes = |gate: Gate| match gate {
        Gate::Always => true,
        Gate::Buildx(a, b, c) => buildx_satisfies(&version, (a, b, c)),
        Gate::SecretFile(i) => written[i],
    };
    let (program, mut args) = buildx_cmd(&[], standalone);
    args.extend(
        spec.args
            .iter()
            .filter(|a| passes(a.gate))
            .map(|a| interpolate(&render(&a.pieces, &r), ctx.env)),
    );
    let (out, code) = merged(&program, &args, root, &penv, &tmp.join("build-output"))
        .map_err(|e| eyre!("spawn {program}: {e}"))?;
    log.push_str(&out);
    // tinyexec throws on a non-zero exit code; a signal leaves it undefined,
    // which the executor lets through.
    if let Some(c) = code.filter(|c| *c != 0) {
        bail!("Process exited with non-zero status ({c})");
    }
    if spec.create_builder {
        let _ = writeln!(log, "Removing builder {builder}");
        let (program, args) = buildx_cmd(&["rm", &builder], standalone);
        log_exec(log, &program, &args, root, &penv)?;
    }

    let image_id = read_lossy(&tmp.join("iidfile"))?.map(|s| js_trim(&s).to_string());
    let metadata = read_lossy(&tmp.join("metadata-file"))?
        .map(|s| js_trim(&s).to_string())
        .filter(|s| s != "null");
    let digest = match &metadata {
        None => None,
        Some(m) => {
            let json: Json =
                serde_json::from_str(m).map_err(|e| eyre!("metadata-file is not JSON: {e}"))?;
            match json.get("containerimage.digest").filter(|d| is_truthy(d)) {
                None => None,
                Some(Json::String(d)) => Some(d.clone()),
                Some(other) => bail!("containerimage.digest is not a string: {other}"),
            }
        }
    };
    let outputs = [
        ("ImageID", "imageid", image_id),
        ("Digest", "digest", digest),
        ("Metadata", "metadata", metadata),
    ];
    for (title, name, value) in outputs {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            let _ = writeln!(log, "{title}\n{v}");
            set_output(root, &spec.project, name, &v)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use serde_json::json;

    use super::*;
    use crate::graph::{Project, ProjectGraph};

    /// zerg_api's `container` target as the workspace graph resolves it.
    fn zerg_api(ci: bool) -> Json {
        let mut o = json!({
            "file": "manifests/dockers/rust.Dockerfile",
            "context": ".",
            "build-args": ["APP_NAME=zerg_api"],
            "target": "rust",
            "tags": ["$REGISTRY/zerg-api:latest"],
            "push": false
        });
        if ci {
            let c = json!({
                "push": true,
                "cache-from": ["type=registry,ref=$BUILDCACHE/zerg-api"],
                "cache-to": ["type=registry,ref=$BUILDCACHE/zerg-api,mode=max,image-manifest=true,oci-mediatypes=true"],
                "metadata": {
                    "images": ["$REGISTRY/zerg-api"],
                    "tags": [
                        "type=sha",
                        "type=ref,event=branch",
                        "type=ref,event=pr",
                        "type=raw,value=$APP_VERSION,enable=$ENABLE_VERSION"
                    ]
                }
            });
            o.as_object_mut()
                .unwrap()
                .extend(c.as_object().unwrap().clone());
        }
        o
    }

    fn env_of(pairs: &[(&str, &str)]) -> Env {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn plan_in(root: &Path, options: Json, env: &Env) -> Result<Plan> {
        let project = Project {
            name: "zerg_api".into(),
            root: "apps/zerg/api".into(),
            project_type: None,
            tags: Vec::new(),
            implicit_dependencies: Vec::new(),
            targets: BTreeMap::new(),
            deps: Default::default(),
            build_deps: Default::default(),
        };
        let graph = ProjectGraph {
            projects: BTreeMap::from([("zerg_api".into(), project)]),
            ..Default::default()
        };
        let options: JsonMap = serde_json::from_value(options).unwrap();
        let overrides = JsonMap::new();
        let ctx = PlanCtx {
            workspace_root: root,
            graph: &graph,
            project: &graph.projects["zerg_api"],
            target: "container",
            configuration: None,
            options: &options,
            overrides: &overrides,
            unparsed: &[],
            env,
        };
        build(&ctx)
    }

    fn label_in(root: &Path, options: Json, env: &[(&str, &str)]) -> Result<String> {
        let plan = plan_in(root, options, &env_of(env))?;
        match plan.steps.as_slice() {
            [Step::Native { label, .. }] => Ok(label.clone()),
            _ => panic!("expected one native step"),
        }
    }

    fn label(options: Json, env: &[(&str, &str)]) -> Result<String> {
        label_in(Path::new("/w"), options, env)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "butler-nx-container-{name}-{}-{:x}",
            std::process::id(),
            random_u64()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const GITHUB: &[(&str, &str)] = &[
        ("GITHUB_ACTIONS", "true"),
        ("GITHUB_REPOSITORY", "yurikrupnik/nx-playground"),
        ("GITHUB_SHA", "0123456789abcdef0123456789abcdef01234567"),
        ("INPUT_GITHUB_TOKEN", "ghs_s3cr3t"),
        ("REGISTRY", "europe-docker.pkg.dev/p/docker"),
        ("BUILDCACHE", "ghcr.io/yurikrupnik/nx-playground/buildcache"),
    ];

    fn github(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        GITHUB.iter().chain(extra).copied().collect()
    }

    #[test]
    fn default_configuration_renders_the_buildx_command() {
        let with_registry = label(
            zerg_api(false),
            &[("REGISTRY", "europe-docker.pkg.dev/p/docker")],
        );
        assert_eq!(
            with_registry.unwrap(),
            "docker buildx build --build-arg APP_NAME=zerg_api --file manifests/dockers/rust.Dockerfile \
             --iidfile '<tmp>/iidfile' --tag europe-docker.pkg.dev/p/docker/zerg-api:latest --target rust \
             --metadata-file '<tmp>/metadata-file' ."
        );
        // An unset variable stays literal, as core `interpolate` leaves it.
        let unset = label(zerg_api(false), &[]).unwrap();
        assert!(
            unset.contains(" --tag '$REGISTRY/zerg-api:latest' "),
            "{unset}"
        );
    }

    #[test]
    fn ci_push_to_main_tags_branch_and_sha_and_hides_the_token() {
        let l = label(
            zerg_api(true),
            &github(&[
                ("GITHUB_EVENT_NAME", "push"),
                ("GITHUB_REF", "refs/heads/main"),
                ("ENABLE_VERSION", "false"),
            ]),
        )
        .unwrap();
        assert_eq!(
            l,
            "docker buildx build --build-arg APP_NAME=zerg_api \
             --cache-from type=registry,ref=ghcr.io/yurikrupnik/nx-playground/buildcache/zerg-api \
             --cache-to type=registry,ref=ghcr.io/yurikrupnik/nx-playground/buildcache/zerg-api,mode=max,image-manifest=true,oci-mediatypes=true \
             --file manifests/dockers/rust.Dockerfile --iidfile '<tmp>/iidfile' \
             --label 'org.opencontainers.image.created=<now>' \
             --label 'org.opencontainers.image.description=<github:description>' \
             --label 'org.opencontainers.image.licenses=<github:license.spdx_id>' \
             --label org.opencontainers.image.revision=0123456789abcdef0123456789abcdef01234567 \
             --label 'org.opencontainers.image.source=<github:html_url>' \
             --label 'org.opencontainers.image.title=<github:name>' \
             --label 'org.opencontainers.image.url=<github:html_url>' \
             --label org.opencontainers.image.version=main \
             --secret 'id=GIT_AUTH_TOKEN,src=<tmp>/secret-1' \
             --tag europe-docker.pkg.dev/p/docker/zerg-api:main \
             --tag europe-docker.pkg.dev/p/docker/zerg-api:sha-0123456 \
             --target rust --metadata-file '<tmp>/metadata-file' --push ."
        );
        assert!(!l.contains("ghs_s3cr3t"));
    }

    #[test]
    fn ci_pull_request_tags_pr_number_and_enabled_version() {
        let dir = scratch("pr");
        let event = dir.join("event.json");
        fs::write(
            &event,
            r#"{"number":42,"pull_request":{"base":{"ref":"main"}}}"#,
        )
        .unwrap();
        let event = event.to_string_lossy().into_owned();
        let env: Vec<(&str, &str)> = github(&[
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_REF", "refs/pull/42/merge"),
            ("ENABLE_VERSION", "true"),
            ("APP_VERSION", "1.4.0"),
        ])
        .into_iter()
        .chain([("GITHUB_EVENT_PATH", event.as_str())])
        .collect();
        let l = label(zerg_api(true), &env).unwrap();
        let tail = "--label org.opencontainers.image.version=pr-42 \
                    --secret 'id=GIT_AUTH_TOKEN,src=<tmp>/secret-1' \
                    --tag europe-docker.pkg.dev/p/docker/zerg-api:pr-42 \
                    --tag europe-docker.pkg.dev/p/docker/zerg-api:1.4.0 \
                    --tag europe-docker.pkg.dev/p/docker/zerg-api:sha-0123456 \
                    --target rust --metadata-file '<tmp>/metadata-file' --push .";
        assert!(l.ends_with(tail), "{l}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ci_needs_the_variables_its_tags_reference() {
        let push = [
            ("GITHUB_EVENT_NAME", "push"),
            ("GITHUB_REF", "refs/heads/main"),
        ];
        let err = label(zerg_api(true), &github(&push)).unwrap_err();
        assert!(
            format!("{err:#}").contains(
                "zerg_api:container: Invalid value for enable attribute: $ENABLE_VERSION"
            ),
            "{err:#}"
        );
        let no_token: Vec<_> = github(&[("ENABLE_VERSION", "false")])
            .into_iter()
            .chain(push)
            .filter(|(k, _)| *k != "INPUT_GITHUB_TOKEN")
            .collect();
        let err = label(zerg_api(true), &no_token).unwrap_err();
        assert!(
            format!("{err:#}").contains("Missing github token"),
            "{err:#}"
        );
    }

    #[test]
    fn input_variables_override_options_with_the_executors_precedence() {
        let l = label(
            zerg_api(false),
            &[
                ("INPUT_ZERG_API_TARGET", "builder"),
                ("INPUT_TARGET", "other"),
                // push/load/pull/no-cache/sbom only read the unprefixed name.
                ("INPUT_ZERG_API_PUSH", "true"),
                ("INPUT_LOAD", "yes"),
                ("INPUT_TAGS", "a:1,b:2\nc:3,\"d,e:4\""),
                ("INPUT_BUILD_ARGS", "A=1,B=2\n\"C=3,4\"\n  D = 5  \n"),
            ],
        )
        .unwrap();
        assert_eq!(
            l,
            "docker buildx build --build-arg A=1,B=2 --build-arg C=3,4 --build-arg 'D = 5' \
             --file manifests/dockers/rust.Dockerfile --iidfile '<tmp>/iidfile' --tag a:1 --tag b:2 \
             --tag c:3 --tag d,e:4 --target builder --load --metadata-file '<tmp>/metadata-file' ."
        );
        let l = label(zerg_api(false), &[("INPUT_TARGET", "other")]).unwrap();
        assert!(l.contains("--target other "), "{l}");
    }

    #[test]
    fn exporters_platforms_and_secrets_follow_the_docker_engine() {
        let l = label(
            json!({"outputs": ["type=local,dest=out"], "platforms": ["linux/amd64", "linux/arm64"],
                   "secrets": ["FOO=bar", "BAD", "=x"], "secret-files": ["NPM=.npmrc"]}),
            &[
                ("INPUT_GITHUB_TOKEN", "tok"),
                ("INPUT_CONTEXT", "{{defaultContext}}/apps"),
            ],
        )
        .unwrap();
        assert_eq!(
            l,
            "docker buildx build --file apps/zerg/api/Dockerfile --output type=local,dest=out \
             --platform linux/amd64,linux/arm64 --secret 'id=FOO,src=<tmp>/secret-1' \
             --secret 'id=NPM,src=<tmp>/secret-2' --secret 'id=GIT_AUTH_TOKEN,src=<tmp>/secret-3' \
             --metadata-file '<tmp>/metadata-file' ./apps"
        );
        // A context outside the default one gets no GIT_AUTH_TOKEN.
        let l = label(
            json!({"context": "apps/x"}),
            &[("INPUT_GITHUB_TOKEN", "tok")],
        )
        .unwrap();
        assert!(
            !l.contains("GIT_AUTH_TOKEN") && l.ends_with(" apps/x"),
            "{l}"
        );
    }

    #[test]
    fn created_builder_is_named_created_and_removed() {
        let l = label(zerg_api(false), &[("INPUT_CREATE_BUILDER", "true")]).unwrap();
        assert!(
            l.starts_with(
                "docker buildx create '--name=zerg_api-<random>' && docker buildx build "
            ),
            "{l}"
        );
        assert!(l.contains(" --builder 'zerg_api-<random>' "), "{l}");
        assert!(
            l.ends_with(" && docker buildx rm 'zerg_api-<random>'"),
            "{l}"
        );
    }

    #[test]
    fn only_the_docker_engine_is_ported() {
        let err = label(json!({"engine": "podman"}), &[]).unwrap_err();
        assert!(format!("{err:#}").contains("zerg_api:container: engine `podman` is not ported"));
        let err = label(json!({}), &[("INPUT_ENGINE", "kaniko")]).unwrap_err();
        assert!(format!("{err:#}").contains("Unsupported Container Engine `kaniko`"));
    }

    #[test]
    fn metadata_tag_types_flavor_and_label_order() {
        let meta = |tags: &str, extra: &[(&'static str, &'static str)]| {
            let mut env = github(&[("GITHUB_EVENT_NAME", "push")]);
            env.push(("INPUT_TAGS", Box::leak(tags.to_string().into_boxed_str())));
            env.extend(extra);
            label(
                json!({"metadata": {"images": ["$REGISTRY/zerg-api"]}}),
                &env,
            )
            .unwrap()
        };
        let tags = |l: &str| -> Vec<String> {
            l.split(" --tag ")
                .skip(1)
                .map(|t| {
                    t.split(' ')
                        .next()
                        .unwrap()
                        .rsplit(':')
                        .next()
                        .unwrap()
                        .to_string()
                })
                .collect()
        };
        let semver = meta(
            "type=semver,pattern={{version}}\ntype=semver,pattern={{major}}.{{minor}}\n\
             type=sha,format=long,prefix=\ntype=ref,event=tag",
            &[("GITHUB_REF", "refs/tags/v1.2.3")],
        );
        assert_eq!(
            tags(&semver),
            [
                "1.2.3",
                "1.2",
                "v1.2.3",
                "0123456789abcdef0123456789abcdef01234567",
                "latest"
            ]
        );
        let pre = meta(
            "type=semver,pattern={{major}}.{{minor}}\ntype=semver,pattern={{raw}}",
            &[("GITHUB_REF", "refs/tags/v1.2.3-rc.1")],
        );
        assert_eq!(tags(&pre), ["1.2.3-rc.1", "v1.2.3-rc.1"]);
        let flavored = meta(
            "type=sha\ntype=raw,value={{branch}}-{{sha}}-{{is_default_branch}},priority=2000\n\
             type=edge,branch=main",
            &[
                ("GITHUB_REF", "refs/heads/main"),
                (
                    "INPUT_FLAVOR",
                    "latest=true\nprefix=p-{{branch}}-,onlatest=true\nsuffix=-s",
                ),
            ],
        );
        assert_eq!(
            tags(&flavored),
            [
                "p-main-main-0123456-false-s",
                "p-main-edge-s",
                "sha-0123456-s",
                "p--branch--latest"
            ]
        );
        // Custom labels merge by key and sort like `localeCompare`.
        let labels = meta(
            "type=raw,value=x",
            &[
                ("GITHUB_REF", "refs/heads/main"),
                (
                    "INPUT_LABELS",
                    "Zeta=1\nalpha=2\norg.opencontainers.image.title=Custom=x\nnolabel\na_b=3\na-b=4\na.b=6",
                ),
            ],
        );
        let keys: Vec<&str> = labels
            .split(" --label ")
            .skip(1)
            .map(|l| l.trim_start_matches('\'').split('=').next().unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "a_b",
                "a-b",
                "a.b",
                "alpha",
                "org.opencontainers.image.created",
                "org.opencontainers.image.description",
                "org.opencontainers.image.licenses",
                "org.opencontainers.image.revision",
                "org.opencontainers.image.source",
                "org.opencontainers.image.title",
                "org.opencontainers.image.url",
                "org.opencontainers.image.version",
                "Zeta"
            ]
        );
        assert!(
            labels.contains(" --label org.opencontainers.image.title=Custom=x "),
            "{labels}"
        );
        // Handlebars escaping happens before tag sanitisation.
        let escaped = meta(
            "type=raw,value={{branch}}&<x>",
            &[("GITHUB_REF", "refs/heads/a=b")],
        );
        assert_eq!(tags(&escaped), ["a-x3D-b-x-"]);
    }

    #[test]
    fn unported_template_features_are_refused() {
        let env = github(&[
            ("GITHUB_EVENT_NAME", "push"),
            ("GITHUB_REF", "refs/heads/main"),
            ("INPUT_TAGS", "type=raw,value={{date 'YYYYMMDD'}}"),
        ]);
        let err = label(json!({"metadata": {"images": ["r/x"]}}), &env).unwrap_err();
        assert!(
            format!("{err:#}").contains("unsupported handlebars expression"),
            "{err:#}"
        );
        let env = github(&[
            ("GITHUB_REF", "refs/heads/main"),
            ("INPUT_TAGS", "type=edge"),
        ]);
        let err = label(json!({"metadata": {"images": ["r/x"]}}), &env).unwrap_err();
        assert!(
            format!("{err:#}").contains("type=edge without branch="),
            "{err:#}"
        );
    }

    #[test]
    fn metadata_outside_ci_reads_the_local_git_repository() {
        let dir = scratch("git");
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("HOME", &dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q", "-b", "feature/Some_Thing"]);
        git(&[
            "-c",
            "user.email=dev@example.com",
            "-c",
            "user.name=dev",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ]);
        git(&["remote", "add", "origin", "https://example.com/o/r.git"]);
        let sha = git(&["rev-parse", "HEAD"]);
        let path = std::env::var("PATH").unwrap();
        let home = dir.to_string_lossy().into_owned();
        let env = [
            ("PATH", path.as_str()),
            ("HOME", home.as_str()),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("ENABLE_VERSION", "false"),
            ("REGISTRY", "r.io"),
            ("BUILDCACHE", "r.io/cache"),
        ];
        let l = label_in(&dir, zerg_api(true), &env).unwrap();
        assert!(
            l.contains(&format!(
                " --tag r.io/zerg-api:feature-Some_Thing --tag r.io/zerg-api:sha-{} ",
                &sha[..7]
            )),
            "{l}"
        );
        assert!(
            l.contains(&format!(
                " --label org.opencontainers.image.revision={sha} "
            )),
            "{l}"
        );
        assert!(
            l.contains(" --label org.opencontainers.image.source=https://example.com/o/r.git "),
            "{l}"
        );
        assert!(
            l.contains(" --label org.opencontainers.image.title= "),
            "{l}"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn detached_head_refs() {
        assert_eq!(
            detached_ref("HEAD, tag: v1.0.0").unwrap(),
            "refs/tags/v1.0.0"
        );
        assert_eq!(
            detached_ref("grafted, HEAD, origin/main, main").unwrap(),
            "refs/heads/main"
        );
        assert_eq!(
            detached_ref("HEAD, pull/7/merge").unwrap(),
            "refs/pull/7/merge"
        );
        assert!(detached_ref("HEAD").is_err());
        assert!(detached_ref("HEAD, weird").is_err());
    }

    #[test]
    fn csv_lists_follow_csv_parse() {
        let relaxed = CsvOpts {
            relax_quotes: true,
            skip_empty_lines: true,
            ..CsvOpts::default()
        };
        assert_eq!(
            csv_parse("a,\"b,c\"\n\nd\r\ne", relaxed).unwrap(),
            [vec!["a", "b,c"], vec!["d\r"], vec!["e"]]
        );
        assert_eq!(
            csv_parse("a\"b,\"c\"d\"", relaxed).unwrap(),
            [vec!["a\"b", "\"c\"d\""]]
        );
        let comments = CsvOpts {
            comment: Some('#'),
            ..relaxed
        };
        assert_eq!(
            csv_parse("a#c\n#full\nb", comments).unwrap(),
            [vec!["a"], vec!["b"]]
        );
        // Tag entries are parsed strictly.
        assert!(parse_tag("type=raw,value=\"x\"").is_err());
        assert!(csv_parse("\"open", relaxed).is_err());
        assert!(is_local_or_tar_exporter(&[" type = local ".into()]).unwrap());
        assert!(is_local_or_tar_exporter(&["dest=out".into()]).unwrap());
        assert!(!is_local_or_tar_exporter(&["type=registry".into()]).unwrap());
        assert!(!is_local_or_tar_exporter(&[]).unwrap());
    }

    #[test]
    fn names_interpolation_and_versions() {
        for (name, constant) in [
            ("zerg_api", "ZERG_API"),
            ("todo-astro-web", "TODO_ASTRO_WEB"),
            ("myApp2Web", "MY_APP2_WEB"),
            ("API", "API"),
            ("_lead x", "LEAD_X"),
            ("a..b--C", "A_BC"),
        ] {
            assert_eq!(constant_name(name), constant, "{name}");
        }
        assert_eq!(
            posix_name("ZERG_API_build-args"),
            "INPUT_ZERG_API_BUILD_ARGS"
        );
        let env = env_of(&[("A", "1"), ("B", "2"), ("C", "")]);
        assert_eq!(
            interpolate("${A}/$B/$C/${}x/$/$A}", &env),
            "1/2/$C/${}x/$/1"
        );
        assert_eq!(
            parse_buildx_version("github.com/docker/buildx v0.17.1-desktop.1 257b1ae").as_deref(),
            Some("0.17.1")
        );
        assert_eq!(
            parse_buildx_version("buildx 1234abc").as_deref(),
            Some("1234abc")
        );
        assert!(buildx_satisfies("0.17.1", (0, 8, 0)));
        assert!(!buildx_satisfies("0.5.1", (0, 6, 0)));
        assert!(!buildx_satisfies("0.17", (0, 6, 0)));
        assert!(buildx_satisfies("1234abc", (9, 9, 9)));
        assert_eq!(
            short_sha(
                "0123456789",
                &env_of(&[("NX_CONTAINER_SHORT_SHA_LENGTH", "4.9")])
            )
            .unwrap(),
            "0123"
        );
    }

    /// A `docker` that answers the executor's probes, records the build's
    /// argv and secrets, and writes the iidfile/metadata-file like buildx.
    const FAKE_DOCKER: &str = r#"#!/bin/sh
[ $# -eq 0 ] && exit 0
case "$1 $2" in
  "buildx ") exit 0;;
  "buildx version") echo "github.com/docker/buildx v$FAKE_VERSION 257b1ae"; exit 0;;
  "buildx build")
    printf '%s\n' "$@" > "$FAKE_ARGS"
    while [ $# -gt 0 ]; do
      case "$1" in
        --iidfile) printf 'sha256:image\n' > "$2";;
        --metadata-file) printf '{"containerimage.digest":"sha256:digest"}' > "$2";;
        --secret) cat "${2#*src=}" >> "$FAKE_SECRETS"; echo >> "$FAKE_SECRETS";;
      esac
      shift
    done
    echo "built"; echo "to stderr" >&2
    exit "${FAKE_EXIT:-0}";;
esac
exit 0
"#;

    fn run_fake(version: &str, exit: &str) -> (PathBuf, StepOutput, String, String) {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("run");
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("docker"), FAKE_DOCKER).unwrap();
        fs::set_permissions(bin.join("docker"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(dir.join("tmp")).unwrap();
        fs::write(dir.join("present.txt"), "from-file").unwrap();
        let s = |p: PathBuf| p.to_string_lossy().into_owned();
        let env = env_of(&[
            ("PATH", &format!("{}:/usr/bin:/bin", s(bin.clone()))),
            ("TMPDIR", &s(dir.join("tmp"))),
            ("FAKE_VERSION", version),
            ("FAKE_EXIT", exit),
            ("FAKE_ARGS", &s(dir.join("args"))),
            ("FAKE_SECRETS", &s(dir.join("secrets"))),
            ("NPM_TOKEN", "npm-secret"),
        ]);
        let options = json!({
            "tags": ["r.io/app:latest"],
            "build-contexts": ["x=../y"],
            "secrets": ["NPM=$NPM_TOKEN"],
            "secret-files": ["MISSING=missing.txt", "PRESENT=present.txt"],
            "quiet": true
        });
        let plan = plan_in(&dir, options, &env).unwrap();
        let Step::Native { run, .. } = &plan.steps[0] else {
            panic!()
        };
        let out = run(&NativeCtx {
            workspace_root: &dir,
            env: &env,
        })
        .unwrap();
        let args = fs::read_to_string(dir.join("args")).unwrap_or_default();
        let secrets = fs::read_to_string(dir.join("secrets")).unwrap_or_default();
        (dir, out, args, secrets)
    }

    #[test]
    fn native_step_builds_writes_outputs_and_cleans_up() {
        let (dir, out, args, secrets) = run_fake("0.17.1", "0");
        assert!(out.success, "{}", out.output);
        assert!(out.output.contains("built\nto stderr\n"), "{}", out.output);
        assert!(
            out.output
                .contains("Warning: secret file missing.txt not found"),
            "{}",
            out.output
        );
        let args: Vec<&str> = args.lines().collect();
        let tmp = dir.join("tmp").to_string_lossy().into_owned();
        assert_eq!(
            &args[..5],
            ["buildx", "build", "--build-context", "x=../y", "--file"]
        );
        assert!(args.iter().any(
            |a| a.starts_with(&format!("{tmp}/docker-build-push-")) && a.ends_with("/iidfile")
        ));
        assert!(args.contains(&"--metadata-file"));
        assert!(!args.iter().any(|a| a.starts_with("id=MISSING")));
        assert!(args.iter().any(|a| a.starts_with("id=PRESENT,src=")));
        assert_eq!(secrets, "npm-secret\nfrom-file\n");
        let outputs = dir.join("node_modules/.cache/nx-container/zerg_api");
        assert_eq!(
            fs::read_to_string(outputs.join("imageid")).unwrap(),
            "sha256:image"
        );
        assert_eq!(
            fs::read_to_string(outputs.join("digest")).unwrap(),
            "sha256:digest"
        );
        assert_eq!(
            fs::read_to_string(outputs.join("metadata")).unwrap(),
            r#"{"containerimage.digest":"sha256:digest"}"#
        );
        assert_eq!(
            fs::read_dir(dir.join("tmp")).unwrap().count(),
            0,
            "temp dir left behind"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn native_step_gates_flags_on_buildx_version_and_fails_on_exit_code() {
        let (dir, out, args, _) = run_fake("0.5.0", "3");
        assert!(!out.success);
        assert!(
            out.output
                .contains("Process exited with non-zero status (3)"),
            "{}",
            out.output
        );
        assert!(
            !args.contains("--build-context") && !args.contains("--metadata-file"),
            "{args}"
        );
        assert!(args.contains("--iidfile"), "{args}");
        assert!(!dir.join("node_modules").exists());
        assert_eq!(
            fs::read_dir(dir.join("tmp")).unwrap().count(),
            0,
            "temp dir left behind"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
