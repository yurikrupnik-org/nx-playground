//! `nx:run-commands` and `nx:run-script` (nx 23).
//!
//! run-commands is where most of nx's CLI-argument behavior lives: unknown
//! options become `--key=value` flags, unparsed CLI args are appended to every
//! command (unless `forwardAllArgs: false` or the command interpolates
//! `{args}` / `{args.x}` itself), a single `command` never runs "in
//! parallel", and each process gets the local `node_modules/.bin` directories
//! in front of `PATH`. All of it is reproduced so a target behaves the same
//! under `butler run-many` and `nx run-many`.

use std::collections::BTreeMap;
use std::path::Path;

use eyre::{Result, bail, eyre};
use serde_json::json;

use super::args::{self, js_string};
use super::env::{load_and_expand, npm_run_path};
use super::schema::combine_options;
use super::{Plan, PlanCtx, Step};
use crate::config::{Json, JsonMap};

/// Options run-commands consumes itself (`propKeys`); everything else is
/// forwarded to the commands.
const PROP_KEYS: &[&str] = &[
    "command",
    "commands",
    "color",
    "no-color",
    "parallel",
    "no-parallel",
    "readyWhen",
    "cwd",
    "args",
    "envFile",
    "__unparsed__",
    "env",
    "usePty",
    "streamOutput",
    "verbose",
    "forwardAllArgs",
    "tty",
];

fn schema() -> Json {
    let colors = json!([
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"
    ]);
    json!({
        "properties": {
            "commands": {"type": "array", "items": {"oneOf": [
                {"type": "object", "properties": {
                    "command": {"type": "string"},
                    "forwardAllArgs": {"type": "boolean"},
                    "prefix": {"type": "string"},
                    "prefixColor": {"type": "string", "enum": colors},
                    "color": {"type": "string", "enum": colors},
                    "bgColor": {"type": "string", "enum": [
                        "bgBlack", "bgRed", "bgGreen", "bgYellow", "bgBlue",
                        "bgMagenta", "bgCyan", "bgWhite"
                    ]}
                }, "additionalProperties": false, "required": ["command"]},
                {"type": "string"}
            ]}},
            "command": {"oneOf": [
                {"type": "array", "items": {"type": "string"}},
                {"type": "string"}
            ], "type": "string"},
            "parallel": {"type": "boolean", "default": true},
            "readyWhen": {"oneOf": [
                {"type": "string"},
                {"type": "array", "items": {"type": "string"}}
            ]},
            "args": {"oneOf": [
                {"type": "array", "items": {"type": "string"}},
                {"type": "string"}
            ]},
            "envFile": {"type": "string"},
            "color": {"type": "boolean", "default": false},
            "cwd": {"type": "string"},
            "env": {"type": "object", "additionalProperties": {"type": "string"}},
            "__unparsed__": {"type": "array", "items": {"type": "string"}, "$default": {"$source": "unparsed"}},
            "forwardAllArgs": {"type": "boolean", "default": true},
            "tty": {"type": "boolean"}
        },
        "additionalProperties": true,
        "oneOf": [{"required": ["commands"]}, {"required": ["command"]}]
    })
}

struct Command {
    command: String,
    forward_all_args: Option<bool>,
}

pub(crate) fn run_commands(ctx: &PlanCtx<'_>) -> Result<Plan> {
    let task = format!("{}:{}", ctx.project.name, ctx.target);
    let opts = combine_options(ctx, &schema())?;

    let ready_when = match opts.get("readyWhen") {
        Some(Json::String(_)) => 1,
        Some(Json::Array(a)) => a.len(),
        _ => 0,
    };
    let mut parallel = opts.get("parallel").and_then(Json::as_bool).unwrap_or(true);

    // normalizeOptions: `command` (joined when an array) replaces `commands`.
    let commands: Vec<Command> = if let Some(c) = opts.get("command").filter(|c| is_truthy_str(c)) {
        let command = match c {
            Json::Array(parts) => parts.iter().map(js_string).collect::<Vec<_>>().join(" "),
            other => js_string(other),
        };
        parallel = ready_when > 0;
        vec![Command {
            command,
            forward_all_args: None,
        }]
    } else {
        let list = opts
            .get("commands")
            .and_then(Json::as_array)
            .ok_or_else(|| eyre!("{task}: run-commands needs `command` or `commands`"))?;
        let mut out = Vec::with_capacity(list.len());
        for c in list {
            match c {
                Json::String(s) => out.push(Command {
                    command: s.clone(),
                    forward_all_args: None,
                }),
                Json::Object(o) => {
                    if ["prefix", "prefixColor", "color", "bgColor"]
                        .iter()
                        .any(|k| o.get(*k).is_some_and(is_truthy_str))
                        && !parallel
                    {
                        bail!(
                            "{task}: Bad executor config for run-commands - \"prefix\", \"prefixColor\", \
                             \"color\" and \"bgColor\" can only be set when \"parallel=true\"."
                        );
                    }
                    out.push(Command {
                        command: o.get("command").map(js_string).unwrap_or_default(),
                        forward_all_args: o.get("forwardAllArgs").and_then(Json::as_bool),
                    });
                }
                other => bail!("{task}: unsupported commands entry {other}"),
            }
        }
        out
    };
    if ready_when > 0 {
        // readyWhen turns the target into a long-running process nx treats
        // as "done" once a string appears; butler's runner only knows tasks
        // that exit.
        bail!(
            "{task}: run-commands `readyWhen` (a continuous task) is not supported by butler; run it through nx"
        );
    }

    let args_opt = match opts.get("args") {
        Some(Json::Array(a)) => Some(a.iter().map(js_string).collect::<Vec<_>>().join(" ")),
        Some(v) if is_truthy_str(v) => Some(js_string(v)),
        _ => None,
    };
    let unparsed: Vec<String> = opts
        .get("__unparsed__")
        .and_then(Json::as_array)
        .map(|a| a.iter().map(js_string).collect())
        .unwrap_or_default();
    let unparsed_args = args::parse(&unparsed, args::RUN_COMMANDS_UNPARSED);
    let unknown: JsonMap = opts
        .iter()
        .filter(|(k, _)| !PROP_KEYS.contains(&k.as_str()) && !unparsed_args.contains_key(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let parsed_args = parse_args(&unparsed_args, &unknown, args_opt.as_deref());
    let forward_default = opts.get("forwardAllArgs").and_then(Json::as_bool);
    let interp = Interp {
        unknown: &unknown,
        parsed_args: &parsed_args,
        args: args_opt.as_deref(),
        unparsed: &unparsed,
    };

    let cwd = calculate_cwd(opts.get("cwd").and_then(Json::as_str));
    let env = command_env(ctx, &opts, &cwd, &task)?;

    let mut steps = Vec::with_capacity(commands.len());
    for c in commands {
        let forward = c.forward_all_args.or(forward_default).unwrap_or(true);
        let script = interp
            .command(&c.command, forward)
            .map_err(|e| eyre!("{task}: {e}"))?;
        steps.push(Step::Shell {
            script,
            cwd: cwd.clone(),
            env: env.clone(),
        });
    }
    Ok(Plan { steps, parallel })
}

fn is_truthy_str(v: &Json) -> bool {
    match v {
        Json::Null | Json::Bool(false) => false,
        Json::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// `calculateCwd`: empty = workspace root, absolute stays absolute, else
/// workspace-relative. The graph already resolved `{workspaceRoot}` /
/// `{projectRoot}` tokens in options.
fn calculate_cwd(cwd: Option<&str>) -> String {
    match cwd {
        None | Some("") => ".".into(),
        Some(c) => c.trim_end_matches('/').to_string(),
    }
}

/// `processEnv`: npm-run-path `PATH`, then `envFile` (never overriding what
/// is already set), then the `env` option over everything, then
/// `FORCE_COLOR` for `color: true`. Only what differs from the task env is
/// returned.
fn command_env(
    ctx: &PlanCtx<'_>,
    opts: &JsonMap,
    cwd: &str,
    task: &str,
) -> Result<BTreeMap<String, String>> {
    let abs_cwd = ctx.workspace_root.join(cwd);
    let mut local = ctx.env.clone();
    let path = ctx.env.get("PATH").map_or("", String::as_str);
    let local_path = npm_run_path(&abs_cwd, path);
    local.insert("PATH".into(), local_path.clone());
    if let Some(file) = opts.get("envFile").and_then(Json::as_str)
        && ctx.env.get("NX_LOAD_DOT_ENV_FILES").map(String::as_str) != Some("false")
    {
        let file = ctx.workspace_root.join(file);
        if !file.exists() {
            bail!("{task}: envFile {} does not exist", file.display());
        }
        load_and_expand(&[file], &mut local)?;
    }
    if let Some(env) = opts.get("env").and_then(Json::as_object) {
        for (k, v) in env {
            local.insert(k.clone(), js_string(v));
        }
    }
    // The `env` option cannot replace PATH: nx restores the npm-run-path one.
    local.insert("PATH".into(), local_path);
    if opts.get("color").and_then(Json::as_bool) == Some(true) {
        local.insert("FORCE_COLOR".into(), "true".into());
    }
    local.remove("NX_PREFIX_OUTPUT");
    Ok(local
        .into_iter()
        .filter(|(k, v)| ctx.env.get(k) != Some(v))
        .collect())
}

/// `parseArgs`: unknown options, then the `args` option (camel-cased
/// yargs), then unparsed CLI args, later winning.
fn parse_args(unparsed_args: &JsonMap, unknown: &JsonMap, args_opt: Option<&str>) -> JsonMap {
    let mut out = unknown.clone();
    if let Some(a) = args_opt {
        let trimmed = a.strip_prefix('"').unwrap_or(a);
        let trimmed = trimmed.strip_suffix('"').unwrap_or(trimmed);
        let words = split_words(trimmed);
        out.extend(args::parse(&words, args::RUN_COMMANDS_ARGS));
    }
    out.extend(unparsed_args.clone());
    out
}

/// yargs-parser's string tokenizer: whitespace-separated, quotes group.
fn split_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c == ' ' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                }
                cur.push(c);
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

struct Interp<'a> {
    unknown: &'a JsonMap,
    parsed_args: &'a JsonMap,
    args: Option<&'a str>,
    unparsed: &'a [String],
}

impl Interp<'_> {
    /// `interpolateArgsIntoCommand`.
    fn command(&self, command: &str, forward_all_args: bool) -> Result<String> {
        let has_named = command.contains("{args.");
        let has_all = command.contains("{args}");
        if has_named && has_all {
            bail!(
                "Command should not contain both {{args}} and {{args.*}} values. Please choose one to use."
            );
        }
        if has_named {
            let mut out = String::new();
            let mut rest = command;
            while let Some(i) = rest.find("{args.") {
                out.push_str(&rest[..i]);
                let after = &rest[i + "{args.".len()..];
                match after.find('}').filter(|n| *n > 0) {
                    Some(n) => {
                        let key = &after[..n];
                        if let Some(v) = self.parsed_args.get(key) {
                            out.push_str(&js_string(v));
                        }
                        rest = &after[n + 1..];
                    }
                    None => {
                        out.push_str("{args.");
                        rest = after;
                    }
                }
            }
            out.push_str(rest);
            return Ok(out);
        }
        if has_all {
            let mut all = self.unknown_args();
            all.extend(self.unparsed_args());
            let args_string = format!("{} {}", all.join(" "), self.args.unwrap_or(""));
            return Ok(command.replace("{args}", &args_string));
        }
        if !forward_all_args {
            return Ok(command.to_string());
        }
        let mut out = command.to_string();
        let unknown = self.unknown_args().join(" ");
        if !unknown.is_empty() {
            out.push(' ');
            out.push_str(&unknown);
        }
        if let Some(a) = self.args.filter(|a| !a.is_empty()) {
            out.push(' ');
            out.push_str(a);
        }
        if !self.unparsed.is_empty() {
            let rest = self.unparsed_args();
            if !rest.is_empty() {
                out.push(' ');
                out.push_str(&rest.join(" "));
            }
        }
        Ok(out)
    }

    /// `unknownOptionsToArgsArray`: primitives only, unless the CLI or
    /// `args` redefined the key.
    fn unknown_args(&self) -> Vec<String> {
        self.unknown
            .iter()
            .filter(|(k, v)| {
                !matches!(v, Json::Object(_) | Json::Array(_) | Json::Null)
                    && self.parsed_args.get(*k) == Some(*v)
            })
            .map(|(k, v)| wrap_arg(&format!("--{k}={}", js_string(v))))
            .collect()
    }

    /// `unparsedOptionsToArgsArray`.
    fn unparsed_args(&self) -> Vec<String> {
        let filtered = filter_prop_keys(self.unparsed, self.parsed_args);
        rejoin_quoted_fragments(filtered)
            .iter()
            .map(|a| wrap_arg(a))
            .collect()
    }
}

/// `filterPropKeysFromUnParsedOptions`: drop run-commands' own options (and
/// the value that followed them) from the forwarded CLI args.
fn filter_prop_keys(unparsed: &[String], parsed: &JsonMap) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < unparsed.len() {
        let element = &unparsed[i];
        if element.starts_with("--") {
            let key = element.replacen("--", "", 1);
            if element.contains('=') {
                let base = key
                    .split('=')
                    .next()
                    .unwrap_or("")
                    .split('.')
                    .next()
                    .unwrap_or("");
                if !PROP_KEYS.contains(&base) {
                    out.push(element.clone());
                }
            } else if PROP_KEYS.contains(&key.as_str()) {
                if let (Some(next), Some(value)) = (unparsed.get(i + 1), parsed.get(&key))
                    && is_truthy_str(value)
                    && *next == js_string(value)
                {
                    i += 1;
                }
            } else {
                out.push(element.clone());
            }
        } else {
            out.push(element.clone());
        }
        i += 1;
    }
    out
}

/// `rejoinQuotedFragments`: re-assemble a quoted value the shell split.
fn rejoin_quoted_fragments(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let quote = arg
            .chars()
            .next()
            .filter(|c| arg.len() > 1 && (*c == '\'' || *c == '"'));
        match quote {
            Some(q) if !arg.ends_with(q) => {
                let mut fragments = vec![arg.clone()];
                let mut found = false;
                i += 1;
                while i < args.len() {
                    fragments.push(args[i].clone());
                    let closes = args[i].ends_with(q);
                    i += 1;
                    if closes {
                        found = true;
                        break;
                    }
                }
                if found {
                    out.push(fragments.join(" "));
                } else {
                    out.extend(fragments);
                }
            }
            _ => {
                out.push(arg.clone());
                i += 1;
            }
        }
    }
    out
}

/// `/[|&;<>()$`\\!"'*?[\]{}~#\s]/`
fn needs_shell_quoting(s: &str) -> bool {
    s.chars().any(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '|' | '&'
                    | ';'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '$'
                    | '`'
                    | '\\'
                    | '!'
                    | '"'
                    | '\''
                    | '*'
                    | '?'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '~'
                    | '#'
            )
    })
}

fn is_already_quoted(s: &str) -> bool {
    s.len() >= 2
        && ((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')))
}

/// `wrapArgIntoQuotesIfNeeded`.
fn wrap_arg(arg: &str) -> String {
    if let Some((key, value)) = arg.split_once('=') {
        if key.starts_with("--") && needs_shell_quoting(value) && !is_already_quoted(value) {
            return format!("{key}=\"{}\"", value.replace('"', "\\\""));
        }
        return arg.to_string();
    }
    if needs_shell_quoting(arg) && !is_already_quoted(arg) {
        return format!("\"{}\"", arg.replace('"', "\\\""));
    }
    arg.to_string()
}

/// `nx:run-script`: the package manager's `run` in the project root, CLI
/// args after `--`, with the workspace's own `node_modules` bin directories
/// removed from `PATH` (the package manager adds them back itself).
pub(crate) fn run_script(ctx: &PlanCtx<'_>) -> Result<Plan> {
    let task = format!("{}:{}", ctx.project.name, ctx.target);
    let schema = json!({
        "properties": {
            "script": {"type": "string"},
            "__unparsed__": {"type": "array", "items": {"type": "string"}, "$default": {"$source": "unparsed"}}
        },
        "additionalProperties": true,
        "required": ["script"]
    });
    let opts = combine_options(ctx, &schema)?;
    let script = opts.get("script").map(js_string).unwrap_or_default();
    let args = opts
        .get("__unparsed__")
        .and_then(Json::as_array)
        .map(|a| a.iter().map(js_string).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let command = match package_manager(ctx.workspace_root, ctx.env)? {
        "bun" => format!("bun run {script} -- {args}"),
        "npm" if args.is_empty() => format!("npm run {script}"),
        "npm" => format!("npm run {script} -- {args}"),
        other => bail!(
            "{task}: nx:run-script under the `{other}` package manager is not ported to butler \
             (its `run` form depends on the installed {other} version); run it through nx"
        ),
    };
    let node_modules = ctx
        .workspace_root
        .join("node_modules")
        .to_string_lossy()
        .into_owned();
    let path = ctx.env.get("PATH").map_or("", String::as_str);
    let filtered: Vec<&str> = path
        .split(':')
        .filter(|p| !p.starts_with(node_modules.as_str()))
        .collect();
    let mut env = BTreeMap::new();
    let filtered = filtered.join(":");
    if filtered != path {
        env.insert("PATH".to_string(), filtered);
    }
    Ok(Plan {
        steps: vec![Step::Shell {
            script: command,
            cwd: ctx.project.root.clone(),
            env,
        }],
        parallel: false,
    })
}

/// nx `detectPackageManager`: `nx.json` `cli.packageManager`, then the
/// workspace lock file, then the invoking package manager's user agent.
fn package_manager(root: &Path, env: &BTreeMap<String, String>) -> Result<&'static str> {
    let nx_json = root.join("nx.json");
    if nx_json.exists() {
        let raw = std::fs::read_to_string(&nx_json)?;
        let v: Json = serde_json::from_str(&raw).map_err(|e| eyre!("parsing nx.json: {e}"))?;
        if let Some(pm) = v.pointer("/cli/packageManager").and_then(Json::as_str) {
            return Ok(match pm {
                "bun" => "bun",
                "npm" => "npm",
                "yarn" => "yarn",
                "pnpm" => "pnpm",
                other => bail!("nx.json cli.packageManager `{other}` is unknown"),
            });
        }
    }
    let has = |f: &str| root.join(f).exists();
    Ok(if has("bun.lockb") || has("bun.lock") {
        "bun"
    } else if has("yarn.lock") {
        "yarn"
    } else if has("pnpm-lock.yaml") {
        "pnpm"
    } else if has("package-lock.json") {
        "npm"
    } else {
        match env.get("npm_config_user_agent").map(String::as_str) {
            Some(ua) if ua.starts_with("pnpm/") => "pnpm",
            Some(ua) if ua.starts_with("yarn/") => "yarn",
            Some(ua) if ua.starts_with("bun/") => "bun",
            _ => "npm",
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Project, ProjectGraph};

    struct Fixture {
        graph: ProjectGraph,
        env: BTreeMap<String, String>,
    }

    fn fixture() -> Fixture {
        let project = Project {
            name: "app".into(),
            root: "apps/app".into(),
            project_type: None,
            tags: vec![],
            implicit_dependencies: vec![],
            targets: BTreeMap::new(),
            deps: Default::default(),
            build_deps: Default::default(),
        };
        Fixture {
            graph: ProjectGraph {
                projects: BTreeMap::from([("app".into(), project)]),
                ..Default::default()
            },
            env: BTreeMap::from([("PATH".into(), "/usr/bin".into())]),
        }
    }

    fn plan(options: Json, cli: &[&str]) -> Result<Plan> {
        let f = fixture();
        let unparsed: Vec<String> = cli.iter().map(|s| (*s).to_string()).collect();
        let mut overrides = args::parse(&unparsed, args::OVERRIDES);
        if overrides["_"].as_array().is_some_and(Vec::is_empty) {
            overrides.remove("_");
        }
        let options: JsonMap = serde_json::from_value(options).unwrap();
        let ctx = PlanCtx {
            workspace_root: Path::new("/w"),
            graph: &f.graph,
            project: &f.graph.projects["app"],
            target: "t",
            configuration: None,
            options: &options,
            overrides: &overrides,
            unparsed: &unparsed,
            env: &f.env,
        };
        run_commands(&ctx)
    }

    fn scripts(p: &Plan) -> Vec<String> {
        p.steps
            .iter()
            .map(|s| match s {
                Step::Shell { script, .. } => script.clone(),
                _ => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn single_command_is_serial_and_gets_cli_args() {
        let p = plan(
            json!({"command": "ruff format src", "cwd": "apps/app"}),
            &["--check"],
        )
        .unwrap();
        assert!(!p.parallel);
        assert_eq!(scripts(&p), vec!["ruff format src --check"]);
        let Step::Shell { cwd, env, .. } = &p.steps[0] else {
            unreachable!()
        };
        assert_eq!(cwd, "apps/app");
        assert!(env["PATH"].starts_with(
            "/w/apps/app/node_modules/.bin:/w/apps/node_modules/.bin:/w/node_modules/.bin:"
        ));
    }

    #[test]
    fn commands_default_parallel_and_honor_forward_all_args() {
        let p = plan(
            json!({"commands": ["a", {"command": "b", "forwardAllArgs": false}]}),
            &["--x=1", "pos"],
        )
        .unwrap();
        assert!(p.parallel);
        assert_eq!(scripts(&p), vec!["a --x=1 pos", "b"]);
    }

    #[test]
    fn unknown_options_and_interpolation() {
        let p = plan(
            json!({"command": "echo", "flag": "a b", "n": 2, "obj": {"k": 1}}),
            &[],
        )
        .unwrap();
        assert_eq!(scripts(&p), vec!["echo --flag=\"a b\" --n=2"]);
        let p = plan(json!({"command": "echo {args.name}!"}), &["--name=world"]).unwrap();
        assert_eq!(scripts(&p), vec!["echo world!"]);
        let p = plan(json!({"command": "echo {args} end"}), &["--a", "--b=2"]).unwrap();
        assert_eq!(scripts(&p), vec!["echo --a --b=2  end"]);
    }

    #[test]
    fn own_options_are_not_forwarded_and_cli_coerces() {
        let p = plan(
            json!({"commands": ["a", "b"]}),
            &["--parallel=false", "--cwd", "x"],
        )
        .unwrap();
        assert!(!p.parallel);
        assert_eq!(scripts(&p), vec!["a", "b"]);
        let Step::Shell { cwd, .. } = &p.steps[0] else {
            unreachable!()
        };
        assert_eq!(cwd, "x");
    }

    #[test]
    fn both_command_and_commands_is_a_schema_error() {
        assert!(plan(json!({"command": "a", "commands": ["b"]}), &[]).is_err());
        assert!(plan(json!({}), &[]).is_err());
    }

    #[test]
    fn env_option_overrides() {
        let p = plan(json!({"command": "x", "env": {"A": "1"}}), &[]).unwrap();
        let Step::Shell { env, .. } = &p.steps[0] else {
            unreachable!()
        };
        assert_eq!(env["A"], "1");
    }
}
