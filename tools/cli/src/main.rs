//! zergctl — local dev CLI + Nx graph engine for an Nx + Cargo monorepo.
//!
//!   zergctl graph nodes --workspace <root> <tiltfiles...>   (used by the Nx plugin shim)
//!   zergctl new <slug> [--kind rust|static] [--port N]      (offline scaffolding)
//!   zergctl ai "<description>" [--kind rust|static]         (AI-assisted scaffolding)

mod ai;
mod config;
mod graph;
mod scaffold;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use eyre::Result;

use scaffold::{AppKind, AppSpec};

#[derive(Parser)]
#[command(
    name = "zergctl",
    about = "Local dev CLI + Nx graph engine for the zerg monorepo"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Nx project-graph integration.
    Graph {
        #[command(subcommand)]
        cmd: GraphCmd,
    },
    /// Scaffold a new app under {apps_dir}/<slug> (no project.json — targets are inferred).
    New {
        /// App slug, e.g. `notifier` or `email-blast`.
        slug: String,
        #[arg(long, value_enum, default_value_t = Kind::Rust)]
        kind: Kind,
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
    /// Describe a service in natural language; AI fills the spec, then scaffolds it.
    Ai {
        /// Natural-language description of the service to create.
        #[arg(required = true, num_args = 1..)]
        prompt: Vec<String>,
        /// Override the kind the model picks.
        #[arg(long, value_enum)]
        kind: Option<Kind>,
    },
}

#[derive(Subcommand)]
enum GraphCmd {
    /// Emit `createNodesV2` JSON for the given Tiltfiles (workspace-relative paths).
    Nodes {
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
        /// Conventions as JSON; the Nx shim forwards its plugin options here.
        #[arg(long)]
        config_json: Option<String>,
        #[arg(required = true, num_args = 1..)]
        files: Vec<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Rust,
    Static,
}

impl From<Kind> for AppKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Rust => AppKind::Rust,
            Kind::Static => AppKind::Static,
        }
    }
}

fn report_scaffold(spec: &AppSpec, written: &[PathBuf]) {
    eprintln!("Created {} ({:?}):", spec.name, spec.kind);
    for p in written {
        eprintln!("  + {}", p.display());
    }
    eprintln!(
        "\nNo project.json needed — Nx infers build/test/lint/run/container/scan.\nTry: nx show project {}",
        spec.name
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    match cli.command {
        Command::Graph {
            cmd:
                GraphCmd::Nodes {
                    workspace,
                    config_json,
                    files,
                },
        } => {
            let conv = config::Conventions::resolve(&workspace, config_json.as_deref())?;
            // stdout MUST be only the JSON the Nx shim parses.
            let nodes = graph::nodes_for(&files, &workspace, &conv);
            println!("{}", serde_json::to_string(&nodes)?);
        }
        Command::New { slug, kind, port } => {
            let ws = std::env::current_dir()?;
            let conv = config::Conventions::resolve(&ws, None)?;
            let spec = AppSpec::from_slug(&slug, kind.into(), port, &conv);
            let written = scaffold::scaffold(&spec, &ws, &conv)?;
            report_scaffold(&spec, &written);
        }
        Command::Ai { prompt, kind } => {
            let ws = std::env::current_dir()?;
            let conv = config::Conventions::resolve(&ws, None)?;
            let prompt = prompt.join(" ");
            let ai_spec = ai::extract_spec(&prompt).await?;
            let spec = ai_spec.into_app_spec(kind.map(Into::into), &conv)?;
            let written = scaffold::scaffold(&spec, &ws, &conv)?;
            report_scaffold(&spec, &written);
        }
    }
    Ok(())
}
