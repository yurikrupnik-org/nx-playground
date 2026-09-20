//! Assembles every discovered project into one self-contained HTML document.

use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use eyre::{Context, Result};

use crate::{
    api::{self, Module},
    discover::{Lang, Project, rel, slugify},
    highlight::escape,
    markdown::{DocContext, IdAllocator, Rendered, TocEntry, normalize, render},
};

const STYLE: &str = include_str!("assets/style.css");
const SCRIPT: &str = include_str!("assets/app.js");

pub struct Site<'a> {
    pub title: &'a str,
    pub root: &'a Path,
    /// Directory the document is written to; all relative links are resolved against it.
    pub out_dir: &'a Path,
    pub projects: &'a [Project],
}

/// A project after its markdown has been rendered.
struct Section {
    anchor: String,
    nav_title: String,
    toc: Vec<TocEntry>,
    html: String,
    group: String,
}

/// Render the whole document. Output is deterministic: no timestamps, no absolute paths.
pub fn render_site(site: &Site<'_>) -> Result<String> {
    let anchors = anchor_map(site.projects);
    let mut ids = IdAllocator::default();
    let mut sections = Vec::new();

    for project in site.projects {
        sections.push(render_project(site, project, &anchors, &mut ids)?);
    }

    let mut out = String::with_capacity(64 * 1024);
    let title = escape(site.title);
    out.push_str("<!doctype html>\n<html lang=\"en\" data-theme=\"dark\">\n<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<meta name=\"generator\" content=\"monodocs\">\n");
    let _ = writeln!(out, "<title>{title}</title>");
    let _ = writeln!(out, "<style>\n{STYLE}</style>");
    out.push_str("</head>\n<body>\n");
    out.push_str("<a class=\"skip\" href=\"#content\">Skip to content</a>\n");

    push_sidebar(&mut out, site, &sections);
    push_main(&mut out, site, &sections);

    let _ = writeln!(out, "<script>\n{SCRIPT}</script>");
    out.push_str("</body>\n</html>\n");
    Ok(out)
}

/// Canonical markdown path -> in-page anchor, so cross-project links become `#anchors`.
fn anchor_map(projects: &[Project]) -> HashMap<PathBuf, String> {
    let mut anchors = HashMap::new();
    for project in projects {
        for (index, doc) in project.docs.iter().enumerate() {
            let anchor = if index == 0 && doc.extra_title.is_none() {
                format!("proj-{}", project.slug)
            } else {
                let stem = doc
                    .extra_title
                    .clone()
                    .unwrap_or_else(|| doc.rel.replace('/', "-"));
                format!("doc-{}-{}", project.slug, slugify(&stem))
            };
            anchors.insert(normalize(&doc.path), anchor);
        }
    }
    anchors
}

fn render_project(
    site: &Site<'_>,
    project: &Project,
    anchors: &HashMap<PathBuf, String>,
    ids: &mut IdAllocator,
) -> Result<Section> {
    let anchor = format!("proj-{}", project.slug);
    let mut body = String::new();
    let mut toc = Vec::new();
    let mut primary_title = project.name.clone();

    for (index, doc) in project.docs.iter().enumerate() {
        let text = fs::read_to_string(&doc.path)
            .wrap_err_with(|| format!("reading {}", doc.path.display()))?;
        let doc_dir = doc.path.parent().unwrap_or(site.root);
        let is_primary = index == 0 && doc.extra_title.is_none();
        let doc_anchor = anchors
            .get(&normalize(&doc.path))
            .cloned()
            .unwrap_or_else(|| anchor.clone());
        // Primary README headings live in the project's namespace (`pg-cli--usage`);
        // supplementary docs get their own (`pg-cli-pg-cli-md--options`).
        let prefix = match &doc.extra_title {
            Some(title) if !is_primary => slugify(&format!("{}-{title}", project.slug)),
            _ => project.slug.clone(),
        };
        let ctx = DocContext {
            doc_dir,
            out_dir: site.out_dir,
            prefix: &prefix,
            section_id: Some(&doc_anchor),
            anchors,
        };
        let Rendered { html, toc: entries } = render(&text, &ctx, ids);

        if is_primary {
            if let Some(first) = entries.first() {
                primary_title = first.text.clone();
            }
            toc.extend(entries);
            let _ = write!(body, "<article class=\"doc\">{html}</article>");
        } else {
            // Supplementary docs (`docs/*.md`) are collapsed; the nav still links into them.
            let label = escape(&doc.rel);
            toc.extend(entries.into_iter().filter(|entry| entry.level <= 2));
            let _ = write!(
                body,
                "<details class=\"extra\" id=\"{doc_anchor}-wrap\"><summary>{label}</summary>\
<article class=\"doc\">{html}</article></details>"
            );
        }
    }

    if project.is_undocumented() {
        let lede = match &project.description {
            Some(description) => format!("<p>{}</p>", escape(description)),
            None => String::new(),
        };
        let _ = write!(
            body,
            "<article class=\"doc\"><h2 id=\"{anchor}\" class=\"doc-h\">{}</h2>{lede}\
<p class=\"missing\">No README.md in <code>{}</code>.</p></article>",
            escape(&project.name),
            escape(&project.rel_dir)
        );
        toc.push(TocEntry {
            level: 1,
            id: anchor.clone(),
            text: project.name.clone(),
        });
    }

    // The API surface rustdoc found, listed inline: a crate's modules, structs, enums and
    // functions are part of what it documents, and a link to a separate tree is not the same as
    // seeing them.
    if let Some((article, entry)) = api_outline(site, project, ids) {
        body.push_str(&article);
        toc.push(entry);
    }

    let mut html = String::new();
    let _ = write!(
        html,
        "<section class=\"project\" data-slug=\"{}\" data-lang=\"{}\" data-name=\"{}\">",
        escape(&project.slug),
        project.lang.slug(),
        escape(&format!(
            "{} {} {} {} {}",
            project.name,
            project.rel_dir,
            project.kind,
            project.lang.label(),
            project.description.as_deref().unwrap_or_default()
        ))
    );
    html.push_str(&chips(site, project));
    html.push_str(&body);
    html.push_str("</section>\n");

    Ok(Section {
        anchor,
        nav_title: primary_title,
        toc,
        html,
        group: project.group.clone(),
    })
}

/// Metadata row shown under each project heading.
fn chips(site: &Site<'_>, project: &Project) -> String {
    let mut out = String::from("<div class=\"chips\">");
    if !project.is_workspace() {
        let _ = write!(
            out,
            "<span class=\"chip lang\" style=\"color:var(--{})\">{}</span>",
            project.lang.slug(),
            escape(project.lang.label())
        );
    }
    let _ = write!(out, "<span class=\"chip\">{}</span>", escape(&project.kind));
    if let Some(version) = &project.version {
        let _ = write!(out, "<span class=\"chip\">v{}</span>", escape(version));
    }
    if project.rel_dir != "." {
        let href = source_link(site, &project.dir);
        let _ = write!(
            out,
            "<a class=\"chip\" href=\"{}\">{}</a>",
            escape(&href),
            escape(&project.rel_dir)
        );
    }
    for target in &project.nx_targets {
        let _ = write!(
            out,
            "<span class=\"chip\">nx {}:{}</span>",
            escape(&project.name),
            escape(target)
        );
    }
    if let Some((href, label)) = api_link(site, project) {
        let _ = write!(
            out,
            "<a class=\"chip api\" href=\"{}\">{}</a>",
            escape(&href),
            escape(label)
        );
    }
    out.push_str("</div>");
    out
}

/// Link to generated API docs when they exist next to the document.
///
/// Rust crates come from `cargo doc` (`api/<crate>/index.html`); other languages plug in by
/// dropping their generator's output in the same place (`api/<project>/index.html`) or in the
/// project's own `docs/api/` directory — typedoc for TypeScript, `kcl doc`, `nu-doc`, etc.
fn api_link(site: &Site<'_>, project: &Project) -> Option<(String, &'static str)> {
    let label = match project.lang {
        Lang::Rust => "cargo doc",
        Lang::TypeScript | Lang::JavaScript => "typedoc",
        Lang::Kcl => "kcl doc",
        Lang::Nushell => "nu doc",
    };
    let mut candidates = Vec::new();
    if let Some(crate_name) = &project.rustdoc_crate {
        candidates.push(site.out_dir.join("api").join(crate_name).join("index.html"));
    }
    candidates.push(
        site.out_dir
            .join("api")
            .join(&project.slug)
            .join("index.html"),
    );
    candidates.push(project.dir.join("docs/api/index.html"));

    let found = candidates.into_iter().find(|path| path.is_file())?;
    Some((source_link(site, &found), label))
}

/// The crate's API surface, listed inline. rustdoc stays the source of truth — every name links
/// into the page it generated — so this document shows *what* a crate exposes and the generated
/// reference shows what each item does.
fn api_outline(
    site: &Site<'_>,
    project: &Project,
    ids: &mut IdAllocator,
) -> Option<(String, TocEntry)> {
    let crate_name = project.rustdoc_crate.as_deref()?;
    let dir = site.out_dir.join("api").join(crate_name);
    let module = api::outline(&dir, &project.name)?;
    let base = escape(&source_link(site, &dir));
    let id = ids.alloc(&format!("{}-api", project.slug));

    let mut out = String::new();
    let _ = write!(
        out,
        "<article class=\"doc api\"><h3 id=\"{id}\" class=\"doc-h\">API\
<a class=\"hash\" href=\"#{id}\" aria-label=\"Permalink\">#</a></h3>\
<p class=\"api-lede\">{} from <code>cargo doc</code> — every name links into \
<a href=\"{base}/{}\">the generated reference</a>.</p>",
        escape(&module.tally()),
        escape(&module.href)
    );
    push_api_module(&mut out, &module, &base);
    out.push_str("</article>");

    Some((
        out,
        TocEntry {
            level: 2,
            id,
            text: "API".to_string(),
        },
    ))
}

/// Items first, then child modules — the shape rustdoc's own module pages use.
fn push_api_module(out: &mut String, module: &Module, base: &str) {
    for group in &module.groups {
        let _ = write!(
            out,
            "<div class=\"api-group\"><h4 class=\"api-kind\">{}</h4><ul class=\"api-list\">",
            escape(&group.label)
        );
        for item in &group.items {
            let _ = write!(
                out,
                "<li><a class=\"api-name\" href=\"{base}/{}\"><code>{}</code></a>",
                escape(&item.href),
                escape(&item.name)
            );
            if let Some(summary) = &item.summary {
                let _ = write!(out, " <span class=\"api-sum\">{}</span>", escape(summary));
            }
            out.push_str("</li>");
        }
        out.push_str("</ul></div>");
    }
    for child in &module.modules {
        let _ = write!(
            out,
            "<details class=\"api-mod\"><summary><a href=\"{base}/{}\"><code>{}</code></a>\
<span class=\"api-meta\">{}</span>",
            escape(&child.href),
            escape(&child.name),
            escape(&child.tally())
        );
        if let Some(summary) = &child.summary {
            let _ = write!(out, "<span class=\"api-sum\">{}</span>", escape(summary));
        }
        out.push_str("</summary><div class=\"api-body\">");
        push_api_module(out, child, base);
        out.push_str("</div></details>");
    }
}

/// Path to a repository file, relative to the rendered document.
fn source_link(site: &Site<'_>, path: &Path) -> String {
    let from = normalize(site.out_dir);
    let target = normalize(path);
    let mut from_parts = from.components().peekable();
    let mut target_parts = target.components().peekable();
    while from_parts.peek().is_some() && from_parts.peek() == target_parts.peek() {
        from_parts.next();
        target_parts.next();
    }
    let ups = from_parts.count();
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_string(), ups).collect();
    parts.extend(target_parts.map(|part| part.as_os_str().to_string_lossy().into_owned()));
    if parts.is_empty() {
        rel(site.root, path)
    } else {
        parts.join("/")
    }
}

fn push_sidebar(out: &mut String, site: &Site<'_>, sections: &[Section]) {
    out.push_str("<aside class=\"sidebar\">\n<div class=\"brand\">");
    let _ = write!(
        out,
        "<span class=\"brand-title\">{}</span>\
<span class=\"brand-sub\">{} projects · one document</span></div>",
        escape(site.title),
        site.projects.len()
    );
    out.push_str(
        "<input id=\"filter\" type=\"search\" placeholder=\"Filter projects  /\" \
autocomplete=\"off\" spellcheck=\"false\">\n<nav id=\"nav\">",
    );

    let mut current_group = String::new();
    let mut open = false;
    for (section, project) in sections.iter().zip(site.projects) {
        if section.group != current_group {
            if open {
                out.push_str("</ul></div>");
            }
            let _ = write!(
                out,
                "<div class=\"nav-group\"><h2>{}</h2><ul>",
                escape(&section.group)
            );
            current_group = section.group.clone();
            open = true;
        }
        let _ = write!(
            out,
            "<li class=\"nav-item\" data-slug=\"{}\"><a href=\"#{}\">\
<span class=\"dot lang-{}\"></span>{}</a>",
            escape(&project.slug),
            escape(&section.anchor),
            if project.is_workspace() {
                "none"
            } else {
                project.lang.slug()
            },
            escape(&section.nav_title)
        );
        let children: Vec<&TocEntry> = section
            .toc
            .iter()
            .filter(|entry| entry.level == 2)
            .collect();
        if !children.is_empty() {
            out.push_str("<ul class=\"nav-sub\">");
            for entry in children {
                let _ = write!(
                    out,
                    "<li><a href=\"#{}\">{}</a></li>",
                    escape(&entry.id),
                    escape(&entry.text)
                );
            }
            out.push_str("</ul>");
        }
        out.push_str("</li>");
    }
    if open {
        out.push_str("</ul></div>");
    }
    out.push_str("</nav>\n</aside>\n");
}

fn push_main(out: &mut String, site: &Site<'_>, sections: &[Section]) {
    let languages: BTreeSet<&str> = site
        .projects
        .iter()
        .filter(|project| !project.is_workspace())
        .map(|project| project.lang.label())
        .collect();
    let documented = site
        .projects
        .iter()
        .filter(|project| !project.is_undocumented())
        .count();

    out.push_str("<main id=\"content\">\n<header class=\"page-head\">");
    let _ = write!(out, "<h1>{}</h1>", escape(site.title));
    let _ = write!(
        out,
        "<p class=\"lede\">Every project README in one page — {documented}/{} documented, \
languages: {}.</p>",
        site.projects.len(),
        escape(&languages.into_iter().collect::<Vec<_>>().join(", "))
    );
    out.push_str("<div class=\"actions\">");
    out.push_str("<button id=\"theme\" type=\"button\">Light theme</button>");
    out.push_str("<button id=\"expand\" type=\"button\">Expand all docs</button>");
    if let Some(index) = api_index(site) {
        let _ = write!(
            out,
            "<a class=\"btn\" href=\"{}\">Rust API (cargo doc)</a>",
            escape(&index)
        );
    }
    out.push_str("</div></header>\n");

    push_summary(out, site, sections);
    out.push_str("<p id=\"empty\" hidden>No project matches that filter.</p>\n");
    for section in sections {
        out.push_str(&section.html);
    }
    out.push_str(
        "<footer class=\"page-foot\">Generated by <code>monodocs</code> from the \
<code>README.md</code> of every project in the workspace. Regenerate with \
<code>just docs-html</code>; the output is deterministic, so it diffs cleanly.</footer>\n",
    );
    out.push_str("</main>\n");
}

fn push_summary(out: &mut String, site: &Site<'_>, sections: &[Section]) {
    out.push_str(
        "<table class=\"summary-table\"><thead><tr><th>Project</th><th>Language</th>\
<th>Kind</th><th>Path</th><th>Docs</th></tr></thead><tbody>",
    );
    for (project, section) in site.projects.iter().zip(sections) {
        let docs = if project.is_undocumented() {
            "<span class=\"missing\">none</span>".to_string()
        } else {
            project
                .docs
                .iter()
                .map(|doc| format!("<code>{}</code>", escape(&doc.rel)))
                .collect::<Vec<_>>()
                .join("<br>")
        };
        let _ = write!(
            out,
            "<tr><td><a href=\"#{}\">{}</a></td><td>{}</td><td>{}</td>\
<td><code>{}</code></td><td>{docs}</td></tr>",
            escape(&section.anchor),
            escape(&project.name),
            if project.is_workspace() {
                "&mdash;".to_string()
            } else {
                escape(project.lang.label())
            },
            escape(&project.kind),
            escape(&project.rel_dir),
        );
    }
    out.push_str("</tbody></table>");
}

/// Entry point of the generated rustdoc tree, if one has been produced.
fn api_index(site: &Site<'_>) -> Option<String> {
    let api = site.out_dir.join("api");
    let index = api.join("index.html");
    if index.is_file() {
        return Some(source_link(site, &index));
    }
    site.projects
        .iter()
        .filter_map(|project| project.rustdoc_crate.as_ref())
        .map(|crate_name| api.join(crate_name).join("index.html"))
        .find(|path| path.is_file())
        .map(|path| source_link(site, &path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::discover;

    fn fixture_site(root: &Path, out_dir: &Path) -> Result<String> {
        let projects = discover(root)?;
        render_site(&Site {
            title: "fixture",
            root,
            out_dir,
            projects: &projects,
        })
    }

    #[test]
    fn source_links_are_relative_to_the_document() {
        let projects = Vec::new();
        let site = Site {
            title: "t",
            root: Path::new("/repo"),
            out_dir: Path::new("/repo/dist/docs"),
            projects: &projects,
        };
        assert_eq!(
            source_link(&site, Path::new("/repo/libs/pg-gen")),
            "../../libs/pg-gen"
        );
    }

    #[test]
    fn polyglot_fixture_renders_every_language() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");
        let html = fixture_site(&root, &root.join("dist"))?;

        for expected in [
            "rust-app",
            "ts-lib",
            "kcl-config",
            "nu-scripts",
            ">Rust<",
            ">TypeScript<",
            ">KCL<",
            ">Nushell<",
        ] {
            assert!(html.contains(expected), "missing {expected} in output");
        }
        // Language-specific highlighting must survive into the page.
        assert!(html.contains("data-lang=\"kcl\""));
        assert!(html.contains("data-lang=\"nushell\""));
        assert!(html.contains("data-lang=\"typescript\""));
        // Undocumented projects are reported, not silently dropped.
        assert!(html.contains("No README.md in <code>no-readme</code>"));
        Ok(())
    }

    #[test]
    fn output_is_deterministic() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");
        let first = fixture_site(&root, &root.join("dist"))?;
        let second = fixture_site(&root, &root.join("dist"))?;
        assert_eq!(first, second);
        Ok(())
    }

    #[test]
    fn cross_project_links_resolve_to_anchors() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");
        let html = fixture_site(&root, &root.join("dist"))?;
        // ts-lib/README.md links to ../rust-app/README.md.
        assert!(
            html.contains("href=\"#proj-rust-app\""),
            "link not rewritten"
        );
        Ok(())
    }

    #[test]
    fn rustdoc_items_are_listed_in_the_document() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");
        // `site/` holds a miniature rustdoc tree for the `rust_app` crate; `dist/` holds none.
        let without = fixture_site(&root, &root.join("dist"))?;
        let with = fixture_site(&root, &root.join("site"))?;

        assert!(
            !without.contains("class=\"api-list\""),
            "no rustdoc tree must mean no API outline"
        );
        for expected in [
            // Every kind rustdoc indexed, each name linking into the page it generated.
            "<h4 class=\"api-kind\">Structs</h4>",
            "<h4 class=\"api-kind\">Enums</h4>",
            "<h4 class=\"api-kind\">Functions</h4>",
            "href=\"api/rust_app/struct.Cli.html\"",
            "href=\"api/rust_app/enum.Mode.html\"",
            "href=\"api/rust_app/fn.main.html\"",
            // Modules nest, and their items keep crate-relative hrefs.
            "href=\"api/rust_app/cache/index.html\"",
            "href=\"api/rust_app/cache/struct.Cache.html\"",
            // rustdoc's one-line summaries come along.
            "Parsed command line &amp; flags.",
            // The section is reachable from the sidebar.
            "href=\"#rust-app-api\"",
        ] {
            assert!(with.contains(expected), "missing {expected} in output");
        }
        Ok(())
    }
}
