//! Builds the clap command tree from the registry, and turns the parsed
//! matches back into a concrete HTTP request.
//!
//! The tree is constructed at run time rather than declared with `#[derive]`.
//! That is the whole point of the design: a route added to an axum router,
//! annotated with `#[utoipa::path]` and exported by its `export_openapi_*`
//! test appears in `x --help` with no Rust written here. The cost is one JSON
//! parse and one tree build per invocation, which is a measured line in
//! `docs/delivery-surface-assets.md`.

use std::collections::{BTreeMap, BTreeSet};

use clap::{Arg, ArgAction, ArgMatches, Command, builder::PossibleValuesParser, value_parser};
use serde_json::{Map, Value};

use crate::model::{Op, Registry};
use crate::spec::{Credential, Scalar, coerce};

/// Verb names the CLI reserves for itself. A resource tag colliding with one
/// of these would be shadowed, so the collision is reported rather than
/// silently losing the operation.
pub const RESERVED: [&str; 2] = ["api", "ui"];

/// Suffix appended to a query parameter whose name collides with a body field
/// in the same command. Writes win the bare name because a write command is
/// mostly body.
const QUERY_SUFFIX: &str = "-query";

pub fn build(registry: &Registry) -> Command {
    let mut root = Command::new("x")
        .version(env!("CARGO_PKG_VERSION"))
        .about("One CLI over every HTTP API in this workspace")
        .long_about(
            "Commands are derived at run time from the OpenAPI documents in \
             docs/openapi/, which are generated from the services' own \
             #[utoipa::path] annotations. `x api list` shows what is loaded; \
             `--spec` points x at any running server's document instead.",
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        .args(global_args());

    let mut verbs: BTreeMap<String, Vec<Command>> = BTreeMap::new();

    // Collect leaves per verb first: a verb subcommand is only constructible
    // once all of its resources are known.
    for ((verb, resource), indices) in registry.groups() {
        if RESERVED.contains(&verb.as_str()) {
            continue;
        }
        verbs
            .entry(verb)
            .or_default()
            .push(resource_command(registry, &resource, &indices));
    }

    for (verb, leaves) in verbs {
        let mut verb_command = Command::new(verb.clone())
            .about(verb_about(&verb))
            .subcommand_required(true)
            .arg_required_else_help(true);
        for leaf in leaves {
            verb_command = verb_command.subcommand(leaf);
        }
        root = root.subcommand(verb_command);
    }

    root.subcommand(
        Command::new("api")
            .about("Inspect the loaded API documents")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(Command::new("list").about("List every API, resource and operation"))
            .subcommand(
                Command::new("show")
                    .about("Print one API's OpenAPI document")
                    .arg(Arg::new("api").required(true).help("API key")),
            ),
    )
    .subcommand(
        Command::new("ui")
            .about("Browse the same APIs in a terminal UI")
            .arg(
                Arg::new("api")
                    .long("api")
                    .help("Start focused on this API")
                    .num_args(1),
            ),
    )
}

fn verb_about(verb: &str) -> String {
    match verb {
        "get" => "Read a collection, or one item by id".to_owned(),
        "create" => "Create an item".to_owned(),
        "update" => "Replace an item".to_owned(),
        "delete" => "Delete an item".to_owned(),
        other => format!("`{other}` operations"),
    }
}

fn global_args() -> Vec<Arg> {
    vec![
        Arg::new("spec")
            .long("spec")
            .short('s')
            .global(true)
            .num_args(1)
            .help("Load one OpenAPI document (path or URL) instead of the built-in registry"),
        Arg::new("base-url")
            .long("base-url")
            .global(true)
            .num_args(1)
            .help("Override the origin of the API being addressed"),
        Arg::new("api")
            .long("api")
            .global(true)
            .num_args(1)
            .help("Disambiguate when two APIs expose the same resource"),
        Arg::new("op")
            .long("op")
            .global(true)
            .num_args(1)
            .help("Disambiguate by operationId"),
        Arg::new("token")
            .long("token")
            .global(true)
            .num_args(1)
            .help("Bearer token; defaults to $X_TOKEN"),
        Arg::new("session")
            .long("session")
            .global(true)
            .num_args(1)
            .value_name("VALUE")
            .help(
                "Session cookie value, sent under the cookie name the document \
                 declares; defaults to $X_SESSION",
            ),
        Arg::new("header")
            .long("header")
            .short('H')
            .global(true)
            .action(ArgAction::Append)
            .num_args(1)
            .value_name("NAME:VALUE")
            .help("Extra request header, repeatable"),
        Arg::new("output")
            .long("output")
            .short('o')
            .global(true)
            .num_args(1)
            .value_parser(PossibleValuesParser::new(["table", "json", "yaml"]))
            .help("Output format; defaults to table on a terminal, json when piped"),
        Arg::new("fields")
            .long("fields")
            .global(true)
            .num_args(1)
            .value_name("A,B,C")
            .help(
                "Project the response to these fields, in this order. Applied \
                 client-side: no handler in this workspace implements ?fields=",
            ),
        Arg::new("timeout")
            .long("timeout")
            .global(true)
            .num_args(1)
            .value_parser(value_parser!(u64).range(1..=3600))
            .help("Request timeout in seconds [default: 30]"),
        Arg::new("dry-run")
            .long("dry-run")
            .global(true)
            .action(ArgAction::SetTrue)
            .help("Print the request that would be sent and exit"),
    ]
}

/// One leaf command, covering every operation that shares a `(verb, resource)`.
fn resource_command(registry: &Registry, resource: &str, indices: &[usize]) -> Command {
    let ops: Vec<&Op> = indices.iter().map(|i| &registry.ops[*i]).collect();
    let max_arity = ops.iter().map(|o| o.arity()).max().unwrap_or(0);

    let about = ops
        .iter()
        .find(|o| !o.summary.is_empty())
        .map(|o| o.summary.clone())
        .unwrap_or_else(|| {
            let ids: Vec<&str> = ops.iter().map(|o| o.operation_id.as_str()).collect();
            ids.join(", ")
        });

    let mut command = Command::new(resource.to_owned()).about(about).after_help({
        let rows: Vec<String> = ops
            .iter()
            .map(|o| {
                format!(
                    "  {:<24} {} {}",
                    o.operation_id,
                    o.method.to_uppercase(),
                    registry.apis[o.api].url(&o.path)
                )
            })
            .collect();
        format!("Operations:\n{}", rows.join("\n"))
    });

    if max_arity > 0 {
        let names: Vec<String> = ops
            .iter()
            .max_by_key(|o| o.arity())
            .map(|o| {
                o.path_params
                    .iter()
                    .map(|p| p.name.to_uppercase())
                    .collect()
            })
            .unwrap_or_default();
        command = command.arg(
            Arg::new("args")
                .num_args(0..=max_arity)
                .value_name(names.join("> <"))
                .help(format!(
                    "Path arguments. None selects the collection operation; \
                     {max_arity} selects the item operation"
                )),
        );
    }

    // Flag names are the union across the group; the selected operation
    // filters them again at execution time.
    let body_names: BTreeSet<&str> = ops
        .iter()
        .flat_map(|o| o.body_fields.iter().map(|f| f.name.as_str()))
        .collect();

    let mut seen: BTreeSet<String> = BTreeSet::new();
    for op in &ops {
        for field in &op.body_fields {
            if !seen.insert(field.name.clone()) {
                continue;
            }
            command = command.arg(flag(
                &field.name,
                &field.name,
                field.scalar,
                field.choices.as_deref(),
                &field.help,
            ));
        }
        for param in &op.query_params {
            let flag_name = if body_names.contains(param.name.as_str()) {
                format!("{}{QUERY_SUFFIX}", param.name)
            } else {
                param.name.clone()
            };
            if !seen.insert(flag_name.clone()) {
                continue;
            }
            command = command.arg(flag(
                &flag_name,
                &param.name,
                param.scalar,
                param.choices.as_deref(),
                &param.help,
            ));
        }
    }

    if ops.iter().any(|o| o.has_body) {
        command = command.arg(
            Arg::new("body")
                .long("body")
                .num_args(1)
                .help("Raw JSON request body; `@file` reads a file, `-` reads stdin"),
        );
    }

    command
}

fn flag(
    flag_name: &str,
    value_name: &str,
    scalar: Scalar,
    choices: Option<&[String]>,
    help: &str,
) -> Arg {
    let mut arg = Arg::new(flag_name.to_owned())
        .long(flag_name.to_owned())
        .value_name(value_name.to_uppercase())
        .help(help.to_owned());

    if scalar == Scalar::Bool {
        // `--flag` means true, `--flag=false` is explicit. `require_equals` is
        // load-bearing: with an optional value and no `=`, clap would swallow
        // the following positional, so `x complete todos --force <id>` would
        // lose the id.
        arg = arg
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value("true");
    } else {
        arg = arg.num_args(1);
    }

    match choices {
        Some(values) => arg.value_parser(PossibleValuesParser::new(values.to_vec())),
        None => arg,
    }
}

/// A fully resolved request, ready for [`crate::exec`].
#[derive(Debug)]
pub struct Invocation {
    pub op: usize,
    pub url: String,
    pub method: String,
    pub query: Vec<(String, String)>,
    pub body: Option<Value>,
    /// What the document says this operation needs; see [`crate::model::Op`].
    pub credentials: Vec<Credential>,
}

/// Resolve `(verb, resource)` matches into one operation and its inputs.
pub fn resolve(
    registry: &Registry,
    verb: &str,
    resource: &str,
    matches: &ArgMatches,
    globals: &crate::Globals,
    read_body: impl Fn(&str) -> eyre::Result<Value>,
) -> eyre::Result<Invocation> {
    let groups = registry.groups();
    let group = groups
        .get(&(verb.to_owned(), resource.to_owned()))
        .ok_or_else(|| eyre::eyre!("no `{verb} {resource}` command"))?;

    // `try_get_many`, not `get_many`: a group whose operations take no path
    // parameters never defines `args`, and `get_many` panics on an undefined
    // id rather than returning None.
    let positionals: Vec<&String> = matches
        .try_get_many::<String>("args")
        .ok()
        .flatten()
        .map(Iterator::collect)
        .unwrap_or_default();

    let index = registry.select(
        group,
        positionals.len(),
        globals.op.as_deref(),
        globals.api.as_deref(),
    )?;
    let op = &registry.ops[index];

    let mut path = op.path.clone();
    for (param, value) in op.path_params.iter().zip(positionals.iter()) {
        path = path.replace(&format!("{{{}}}", param.name), value);
    }

    let body_names: BTreeSet<&str> = op.body_fields.iter().map(|f| f.name.as_str()).collect();

    let mut query = Vec::new();
    for param in &op.query_params {
        let flag_name = if body_names.contains(param.name.as_str()) {
            format!("{}{QUERY_SUFFIX}", param.name)
        } else {
            param.name.clone()
        };
        let Some(raw) = matches.get_one::<String>(&flag_name) else {
            continue;
        };
        let value = coerce(&flag_name, raw, param.scalar)?;
        query.push((param.name.clone(), crate::spec::query_value(&value)));
    }

    let body = if op.has_body {
        match matches.get_one::<String>("body") {
            Some(raw) => Some(read_body(raw)?),
            None if op.opaque_body => {
                return Err(eyre::eyre!(
                    "{} takes a request body that is not a flat object; pass it with --body",
                    op.operation_id
                ));
            }
            None => {
                let mut object = Map::new();
                for field in &op.body_fields {
                    let Some(raw) = matches.get_one::<String>(&field.name) else {
                        continue;
                    };
                    object.insert(field.name.clone(), coerce(&field.name, raw, field.scalar)?);
                }
                let missing: Vec<&str> = op
                    .body_fields
                    .iter()
                    .filter(|f| f.required && !object.contains_key(&f.name))
                    .map(|f| f.name.as_str())
                    .collect();
                if !missing.is_empty() {
                    return Err(eyre::eyre!(
                        "{} requires --{}",
                        op.operation_id,
                        missing.join(" --")
                    ));
                }
                Some(Value::Object(object))
            }
        }
    } else {
        None
    };

    let api = &registry.apis[op.api];
    let url = match globals.base_url.as_deref() {
        Some(origin) => {
            let mut overridden = api.clone();
            overridden.origin = origin.to_owned();
            overridden.url(&path)
        }
        None => api.url(&path),
    };

    Ok(Invocation {
        op: index,
        url,
        method: op.method.to_uppercase(),
        query,
        body,
        credentials: op.credentials.clone(),
    })
}
