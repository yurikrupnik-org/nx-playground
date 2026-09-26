//! `x` entry point: resolve the registry, build the command tree from it,
//! dispatch.

use std::process::ExitCode;

use x_cli::{
    Globals, api_list, builtin_registry, command, exec::Transport, read_body, remote_registry,
    render, ui,
};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("x: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> eyre::Result<()> {
    let argv: Vec<String> = std::env::args().collect();

    // The registry decides which subcommands exist, so it is resolved before
    // the tree is built. See `prescan_spec`.
    let registry = match x_cli::prescan_spec(argv.iter().skip(1)) {
        Some(source) => remote_registry(&source).await?,
        None => builtin_registry()?,
    };

    let matches = command::build(&registry).get_matches();
    let globals = Globals::from_matches(&matches)?;

    let Some((verb, sub)) = matches.subcommand() else {
        // `subcommand_required` makes this unreachable.
        return Err(eyre::eyre!("no command"));
    };

    match verb {
        "api" => {
            match sub.subcommand() {
                Some(("list", _)) => print!("{}", api_list(&registry)),
                Some(("show", args)) => {
                    let key = args.get_one::<String>("api").expect("required");
                    let api = registry
                        .apis
                        .iter()
                        .find(|a| a.key == *key)
                        .ok_or_else(|| {
                            let keys: Vec<&str> =
                                registry.apis.iter().map(|a| a.key.as_str()).collect();
                            eyre::eyre!("no API `{key}`; loaded: {}", keys.join(", "))
                        })?;
                    println!("{}", serde_json::to_string_pretty(&api.doc.paths)?);
                }
                _ => return Err(eyre::eyre!("unknown `api` subcommand")),
            }
            return Ok(());
        }
        "ui" => {
            let focus = sub.get_one::<String>("api").cloned();
            return ui::run(&registry, focus, &globals).await;
        }
        _ => {}
    }

    let Some((resource, args)) = sub.subcommand() else {
        return Err(eyre::eyre!("`{verb}` needs a resource"));
    };

    let invocation = command::resolve(&registry, verb, resource, args, &globals, read_body)?;

    if globals.dry_run {
        let query = if invocation.query.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = invocation
                .query
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            format!("?{}", pairs.join("&"))
        };
        let credentials = if invocation.credentials.is_empty() {
            "(none declared)".to_owned()
        } else {
            invocation
                .credentials
                .iter()
                .map(x_cli::spec::Credential::describe)
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!(
            "{} {}{}\ncredential: {}\n{}",
            invocation.method,
            invocation.url,
            query,
            credentials,
            invocation
                .body
                .as_ref()
                .map(|b| serde_json::to_string_pretty(b).unwrap_or_default())
                .unwrap_or_else(|| "(no body)".to_owned())
        );
        return Ok(());
    }

    let transport = Transport::new(
        globals.timeout,
        globals.token.clone(),
        globals.session.clone(),
        globals.headers.clone(),
    )?;
    let outcome = transport.send(&invocation).await?;

    match outcome.body {
        None => {
            // 204 and friends: say what happened rather than printing nothing.
            eprintln!("{} (no content)", outcome.status);
        }
        Some(body) => {
            let body = match &globals.fields {
                Some(fields) => render::project(body, fields),
                None => body,
            };
            println!(
                "{}",
                render::render(&body, globals.format, globals.fields.as_deref())?
            );
        }
    }

    Ok(())
}
