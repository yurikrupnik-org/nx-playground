# monodocs

Renders every project `README.md` in the workspace into **one self-contained HTML document** —
sidebar navigation, filter, per-project metadata, syntax-highlighted code, and, for every Rust
crate whose rustdoc has been generated, the crate's modules, structs, enums and functions listed
inline and linked into `cargo doc`. Discovery is polyglot: Rust crates, TypeScript/JavaScript
packages, KCL modules and Nushell modules are found by their own manifests, so a non-Rust project
added tomorrow appears in the document with no change here.

- **Path:** `apps/monodocs/cli`
- **Kind:** bin (`monodocs`)
- **Nx project:** `monodocs` (`scope:monodocs`, `rust`; `build`/`test`/`lint`/`run` are inferred
  by `tools/nx/plugin.ts`, so there is no `project.json`)

## Usage

```sh
just docs-html                 # dist/docs/index.html (gitignored — a build artifact, not a diff)
just docs-html-api             # same, plus cargo doc under dist/docs/api
just docs-open                 # render, then open it
just docs-list                 # what was discovered, and which markdown each project contributes
just docs-lint                 # dead links, unknown fences, undocumented projects
just docs-lint --fix           # ... applying the formatting subset first
```

| Flag (`build`) | Default | Meaning |
|---|---|---|
| `--root <DIR>` | `.` | Workspace root to scan |
| `-o, --out <FILE>` | `dist/docs/index.html` | Output file; its directory anchors every relative link |
| `--title <STR>` | workspace directory name | Document title |
| `--cargo-doc` | off | Run `cargo doc --workspace --no-deps` first, copy it to `<out>/api`, link it per crate, and list every crate's items inline |
| `--kcl-doc` | off | Run `kcl doc generate --format html` per KCL package into `<out>/api/<package>` |
| `--api-docs` | off | Every available generator (`--cargo-doc` and `--kcl-doc`) |
| `--check` | off | Render but do not write; fail if the file on disk differs |

## Formatting and linting the markdown

The document is only as good as the READMEs it is built from, so the same binary that renders
them keeps them honest.

```sh
monodocs fmt                      # rewrite every discovered document
monodocs fmt --check              # rewrite nothing; list what would change, exit non-zero
monodocs fmt README.md docs/      # explicit files or directories instead of discovery
monodocs lint                     # report; exit non-zero when anything is found
monodocs lint --fix               # repair the formatting subset first, then report the rest
```

`fmt` is deliberately small — it normalises what nobody wants to argue about and never reflows
prose: trailing whitespace, leading blank lines, one trailing newline, runs of 3+ blank lines,
`#`-run spacing in ATX headings, and `*`/`+` unordered bullets rewritten to `-`. Fenced code is
copied through byte for byte (a formatter that edits code samples is one nobody runs), as are
ordered lists and thematic breaks. It is idempotent by construction.

`lint` reports `path:line: message` for the things a renderer cannot fix for you:

| Rule | Message |
|---|---|
| project without a `README.md` | ``project `no-readme` has no README.md`` |
| relative link or image that does not resolve | `link target does not exist: ./missing.md` |
| `#anchor` matching no heading in the target document | ``no heading in docs/guide.md matches `#nowhere` `` |
| `#anchor` matching no heading in this document | ``no heading in this document matches `#absent` `` |
| fence with no language, or one the highlighter does not know | ``fence language `brainfuck` is not highlighted`` |
| markdown that `fmt` would rewrite | ``formatting differs — run `monodocs fmt` `` |

Links with a scheme (`https:`, `mailto:`) are never checked, and `text`/`plain` fences are a
deliberate "do not highlight this", not a typo. `--fix` only ever applies the `fmt` rewrites:
the remaining rules need a human, so they are reported and the command still exits non-zero.

### `.monodocsignore`

Generated markdown must not be reformatted: rewriting a file that a generator owns only makes the
next generator run produce a diff. `<root>/.monodocsignore` lists the workspace-relative paths
`fmt` and `lint` skip, one per line, `#` for comments and a trailing `/` for a whole directory.
No recipe points at `monodocs fmt` any more: markdown normalisation for this workspace is
rumdl's (`.rumdl.toml`, `just fmt-docs`), because two formatters over one file fight. The `fmt`
subcommand and this ignore file remain the engine behind `lint --fix`. Ignored files are still
rendered into the document; they are out of scope for rewriting only.

## Discovery

| Manifest | Language | Name / version / description from |
|---|---|---|
| `Cargo.toml` with `[package]` | Rust | the `[package]` table; kind inferred from `[[bin]]`/`[lib]` and `src/{main,lib}.rs` |
| `package.json` | TypeScript when `tsconfig.json` or `*.ts` is present, else JavaScript | `name`, `version`, `description` |
| `kcl.mod` | KCL | `[package]` table |
| `nupm.nuon`, or any `*.nu` | Nushell | NUON `name`/`version`/`description`, else the directory name |

A virtual workspace `Cargo.toml` (no `[package]`) is not a project. `target/`, `dist/`,
`node_modules/`, `src/`, `tests/`, `fixtures/` and dotted directories are never descended into.
Projects are grouped `Workspace` → `Apps` → `Libraries`, and each project also lists the Nx targets
declared in its `project.json`.

Each project contributes its `README.md` plus any `docs/*.md` beside it (collapsed under a
`<details>`). Root-level `docs/<project>.md` is attributed to the project it names, which is how
`docs/grpc.md` would land inside a `grpc` section rather than the workspace one. A project without
a `README.md` still gets a section, marked as undocumented — gaps are visible, not silent.

## Rendering

- **Single file.** CSS and JavaScript are inlined; local images become `data:` URIs. Nothing is
  fetched at view time, so the document can be mailed, archived or served from anywhere.
- **Deterministic.** No timestamps and no absolute paths, so `--check` can gate CI and the output
  diffs cleanly.
- **Cross-document links.** A relative link to another project's `README.md` becomes an in-page
  anchor; any other repo-relative link is rewritten relative to the output file, so it still opens
  the real source.
- **Headings.** Demoted one level (the page owns `<h1>`), given ids namespaced by project, and
  collected into the sidebar.
- **UI.** Filter box (`/` focuses, `Esc` clears) narrows both nav and body; scroll-spy highlights
  the current section; per-block copy buttons; dark/light toggle persisted in `localStorage`;
  print stylesheet expands collapsed docs for PDF export.

### Syntax highlighting

Fences are tokenised in-process — no syntect, no highlight.js, no CDN — because the two languages
this repo cares most about after Rust (**KCL** and **Nushell**) have no off-the-shelf grammar.
Supported fence languages:

`rust` · `typescript`/`ts`/`tsx`/`js` · `kcl` · `nu`/`nushell` · `bash`/`sh`/`console`/`just` ·
`yaml` · `toml` · `json`/`nuon` · `sql`

Unknown languages are escaped and emitted verbatim, never dropped. Adding one means adding a
`LangSpec` row in `src/highlight.rs` (keywords, comment markers, quotes, and a few behaviour flags
such as `$var` sigils, `--flag` runs or `key:` mappings).

### API documentation

`--cargo-doc` runs rustdoc for the workspace and copies it to `<out>/api`. Every Rust crate then
gets a `cargo doc` chip pointing at `api/<crate>/index.html`, the header gets a workspace-wide
link, and the crate's section gains an **API** block: modules, structs, enums, traits, functions
and macros, each name linking to the page rustdoc generated for it and carrying rustdoc's own
one-line summary. Child modules nest, collapsed, each with its own tally.

That block is read back out of the generated tree — `sidebar-items.js`, the JSON index rustdoc
writes for its own sidebar, for the items; the module's `index.html` for the summaries. rustdoc
stays the single source of truth: there is no second Rust parser here to drift from the compiler,
and private items show up for binaries exactly as rustdoc decided they should. No rustdoc tree
means no block, and markup rustdoc changes later costs the summaries, never the item list.

`--kcl-doc` does the same per KCL package: `kcl doc generate --format html` into
`<out>/api/<package>/`, where the generator's own `<package>.html` is copied to `index.html` (a
copy, not a rename, so its internal links keep resolving). `--api-docs` runs both; rustdoc goes
first because it owns the whole `api/` tree.

Generators for other languages plug into the same slot — write typedoc/`nu` output to
`<out>/api/<project>/index.html` or to the project's own `docs/api/index.html` and the chip appears
with the right label.

## Development

```sh
cargo run -q -p monodocs -- build            # or: bun nx run monodocs:build
cargo nextest run -p monodocs                # lexer, link rewriting, two fixture workspaces
cargo clippy -p monodocs --all-targets -- -D warnings
cargo run -p monodocs -- build --root apps/monodocs/cli/tests/fixtures/polyglot --out /tmp/fixture.html
cargo run -p monodocs -- lint --root apps/monodocs/cli/tests/fixtures/messy   # every rule, firing
```

`tests/fixtures/polyglot` is a miniature workspace holding a Rust, TypeScript, KCL and Nushell
project plus one crate with no README; the render tests assert all four languages, the rewritten
cross-project link, the undocumented marker, and byte-for-byte determinism. `tests/fixtures/messy`
is its opposite: one document that trips every lint rule exactly once at a known line, next to a
clean supplementary doc and a package with no README, so a false positive fails the test as loudly
as a missed finding.
