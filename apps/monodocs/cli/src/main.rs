//! `monodocs` — render every project `README.md` in a polyglot monorepo into one
//! self-contained HTML document, optionally alongside generated API documentation, and keep the
//! markdown it is built from formatted and free of dead references.
//!
//! ```sh
//! monodocs build                      # dist/docs/index.html
//! monodocs build --cargo-doc          # also runs rustdoc into dist/docs/api and links it
//! monodocs build --kcl-doc            # same for `kcl doc` output, per KCL package
//! monodocs build --api-docs           # every available generator
//! monodocs build --check              # fail if the checked-in document is stale
//! monodocs list                       # what was discovered, and from which manifest
//! monodocs fmt [--check] [PATHS…]     # normalise the markdown (rustfmt for prose)
//! monodocs lint [--fix]               # dead links, unknown fences, undocumented projects
//! ```

mod api;
mod discover;
mod fmt;
mod highlight;
mod lint;
mod markdown;
mod render;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, bail};
use eyre::Context;

use crate::{
    discover::{Lang, Project, discover},
    render::{Site, render_site},
};

#[derive(Parser)]
#[command(
    name = "monodocs",
    about = "Render every project README in the workspace into one HTML document"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Render the single-file HTML document
    Build {
        /// Workspace root to scan
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Output HTML file
        #[arg(short, long, default_value = "dist/docs/index.html")]
        out: PathBuf,

        /// Document title (defaults to the workspace directory name)
        #[arg(long)]
        title: Option<String>,

        /// Run `cargo doc --workspace --no-deps` first and link the result from every crate
        #[arg(long)]
        cargo_doc: bool,

        /// Run `kcl doc generate` first and link the result from every KCL package
        #[arg(long)]
        kcl_doc: bool,

        /// Run every available API-doc generator (`--cargo-doc` and `--kcl-doc`)
        #[arg(long)]
        api_docs: bool,

        /// Do not write; fail when the existing file differs from what would be generated
        #[arg(long)]
        check: bool,
    },

    /// List the projects that would be documented
    List {
        /// Workspace root to scan
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },

    /// Normalise markdown in place: trailing whitespace, blank runs, heading and bullet markers
    Fmt {
        /// Workspace root to scan when no paths are given
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Rewrite nothing; list the files that would change and exit non-zero
        #[arg(long)]
        check: bool,

        /// Markdown files or directories to format; defaults to every discovered document
        paths: Vec<PathBuf>,
    },

    /// Check the documentation: dead links and anchors, unhighlightable fences, missing READMEs
    Lint {
        /// Workspace root to scan
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Repair what is mechanically fixable (the `fmt` rewrites) before reporting
        #[arg(long)]
        fix: bool,
    },
}

/// Which API-doc generators `build` should run before rendering.
#[derive(Clone, Copy)]
struct ApiDocs {
    cargo: bool,
    kcl: bool,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    match Cli::parse().command {
        Commands::Build {
            root,
            out,
            title,
            cargo_doc,
            kcl_doc,
            api_docs,
            check,
        } => build(
            &root,
            &out,
            title.as_deref(),
            ApiDocs {
                cargo: cargo_doc || api_docs,
                kcl: kcl_doc || api_docs,
            },
            check,
        ),
        Commands::List { root } => list(&root),
        Commands::Fmt { root, check, paths } => fmt::run(&root, check, &paths),
        Commands::Lint { root, fix } => lint::run(&root, fix),
    }
}

fn build(root: &Path, out: &Path, title: Option<&str>, api: ApiDocs, check: bool) -> Result<()> {
    let root = fs::canonicalize(root).wrap_err_with(|| format!("resolving {}", root.display()))?;
    let out = absolute(out)?;
    let out_dir = out
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&out_dir).wrap_err_with(|| format!("creating {}", out_dir.display()))?;

    let projects = discover(&root)?;
    if projects.is_empty() {
        bail!("no projects found under {}", root.display());
    }

    // rustdoc owns the whole `api/` tree (it writes its own index and shared assets), so it runs
    // first and every other generator drops its output into a subdirectory beside it.
    if api.cargo {
        generate_cargo_doc(&root, &out_dir)?;
    }
    if api.kcl {
        generate_kcl_doc(&out_dir, &projects)?;
    }

    let title = title
        .map(str::to_string)
        .unwrap_or_else(|| directory_name(&root));

    let html = render_site(&Site {
        title: &title,
        root: &root,
        out_dir: &out_dir,
        projects: &projects,
    })?;

    if check {
        let current = fs::read_to_string(&out).unwrap_or_default();
        if current != html {
            bail!(
                "{} is stale — regenerate with `monodocs build`",
                display_rel(&root, &out)
            );
        }
        eprintln!("{} is up to date", display_rel(&root, &out));
        return Ok(());
    }

    fs::write(&out, &html).wrap_err_with(|| format!("writing {}", out.display()))?;
    let undocumented: Vec<&Project> = projects
        .iter()
        .filter(|project| project.is_undocumented())
        .collect();
    eprintln!(
        "{} — {} projects, {} KiB",
        display_rel(&root, &out),
        projects.len(),
        html.len() / 1024
    );
    for project in undocumented {
        eprintln!("  no README.md: {}", project.rel_dir);
    }
    Ok(())
}

fn list(root: &Path) -> Result<()> {
    let root = fs::canonicalize(root).wrap_err_with(|| format!("resolving {}", root.display()))?;
    let projects = discover(&root)?;
    let width = projects
        .iter()
        .map(|project| project.name.len())
        .max()
        .unwrap_or(4);
    for project in &projects {
        let docs = if project.docs.is_empty() {
            "-".to_string()
        } else {
            project
                .docs
                .iter()
                .map(|doc| doc.rel.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!(
            "{:<width$}  {:<10}  {:<17}  {:<28}  {}",
            project.name,
            project.lang.label(),
            project.kind,
            project.rel_dir,
            docs,
        );
    }
    Ok(())
}

/// Run rustdoc for the workspace and place it at `<out_dir>/api`.
fn generate_cargo_doc(root: &Path, out_dir: &Path) -> Result<()> {
    // Build artifacts stay in the Cargo target tree; only `api/` lands next to the document.
    let target_dir = root.join("dist/target/monodocs-doc");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    eprintln!("running cargo doc --workspace --no-deps");
    let status = Command::new(&cargo)
        .current_dir(root)
        .args(["doc", "--workspace", "--no-deps", "--target-dir"])
        .arg(&target_dir)
        .status()
        .wrap_err_with(|| format!("spawning {cargo}"))?;
    if !status.success() {
        bail!("cargo doc failed with {status}");
    }

    let generated = target_dir.join("doc");
    if !generated.is_dir() {
        bail!("cargo doc produced no {}", generated.display());
    }
    let api = out_dir.join("api");
    if api.exists() {
        fs::remove_dir_all(&api).wrap_err_with(|| format!("clearing {}", api.display()))?;
    }
    copy_dir(&generated, &api)?;
    Ok(())
}

/// Run `kcl doc generate` for every KCL package and place each package's HTML at
/// `<out_dir>/api/<project>/index.html` — exactly where `render::api_link` looks, so the
/// `kcl doc` chip lights up with no change to the renderer.
fn generate_kcl_doc(out_dir: &Path, projects: &[Project]) -> Result<()> {
    let packages: Vec<&Project> = projects
        .iter()
        .filter(|project| project.lang == Lang::Kcl)
        .collect();
    if packages.is_empty() {
        eprintln!("--kcl-doc: no KCL packages found, nothing to generate");
        return Ok(());
    }

    // `kcl doc generate --target DIR` always writes DIR/docs/<package>.html, so each package is
    // generated into a scratch directory and then moved to its final name.
    let staging = out_dir.join("api/.kcl-doc");
    for project in packages {
        eprintln!("running kcl doc generate for {}", project.rel_dir);
        if staging.exists() {
            fs::remove_dir_all(&staging)
                .wrap_err_with(|| format!("clearing {}", staging.display()))?;
        }
        fs::create_dir_all(&staging).wrap_err_with(|| format!("creating {}", staging.display()))?;

        let status = Command::new("kcl")
            .current_dir(&project.dir)
            .args(["doc", "generate", "--format", "html", "--target"])
            .arg(&staging)
            .status()
            .wrap_err("spawning kcl — `--kcl-doc` needs the kcl binary on PATH")?;
        if !status.success() {
            bail!(
                "kcl doc generate failed for {} with {status}",
                project.rel_dir
            );
        }

        let generated = staging.join("docs");
        if !generated.is_dir() {
            bail!(
                "kcl doc generate produced no {} for {}",
                generated.display(),
                project.rel_dir
            );
        }
        let dest = out_dir.join("api").join(&project.name);
        if dest.exists() {
            fs::remove_dir_all(&dest).wrap_err_with(|| format!("clearing {}", dest.display()))?;
        }
        copy_dir(&generated, &dest)?;
        ensure_index(&dest)?;
    }
    if staging.exists() {
        fs::remove_dir_all(&staging).wrap_err_with(|| format!("clearing {}", staging.display()))?;
    }
    Ok(())
}

/// `kcl doc` names its entry page after the package (`kcl-config.html`). Copy it to `index.html`
/// — a copy, not a rename, so the generator's own cross-page links keep resolving.
fn ensure_index(dir: &Path) -> Result<()> {
    let index = dir.join("index.html");
    if index.exists() {
        return Ok(());
    }
    let mut pages: Vec<PathBuf> = fs::read_dir(dir)
        .wrap_err_with(|| format!("reading {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .collect();
    pages.sort();
    let Some(entry) = pages.first() else {
        bail!("{} holds no HTML page to link as index.html", dir.display());
    };
    fs::copy(entry, &index).wrap_err_with(|| format!("writing {}", index.display()))?;
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).wrap_err_with(|| format!("creating {}", to.display()))?;
    for entry in fs::read_dir(from).wrap_err_with(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)
                .wrap_err_with(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Absolute path without requiring the file to exist yet.
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().wrap_err("resolving current directory")?;
    Ok(cwd.join(path))
}

fn directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".to_string())
}

fn display_rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|rel| rel.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.display().to_string())
}
