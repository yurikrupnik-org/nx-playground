# Documentation recipes — imported by the root justfile.
#
# `monodocs` is an EXTERNAL tool: it lives in yurikrupnik/wasm-and-k8s and ships
# to crates.io, so this workspace consumes the published binary instead of
# vendoring a copy that drifts (it had already diverged in four files by the time
# the duplicate was removed). `just docs-install` installs the pinned version;
# every recipe below assumes it is on PATH.
#
# It discovers every project in the workspace by its manifest (Cargo.toml with a
# [package], package.json, kcl.mod, nupm.nuon) and renders the project READMEs
# plus docs/*.md into ONE self-contained HTML file: no CDN, no JS bundler, no
# server — open the file. Output goes to `dist/`, which is gitignored, so the
# document is a build artifact and never a diff.
#
# `docs-lint` is deliberately NOT in `just verify`: 36 of the 59 discovered
# projects have no README, so the gate is red by construction today. Wire it in
# once that backlog is closed; until then it is the worklist itself.

# Pinned so every machine and CI run renders with the same generator — the
# output is byte-deterministic, which is worth nothing if the binary floats.
monodocs_version := "0.1.0"

# Install/refresh the pinned monodocs (from crates.io, not this workspace)
[group('docs')]
docs-install:
    cargo install monodocs --locked --version {{ monodocs_version }}

# Render every README into dist/docs/index.html
[group('docs')]
docs-html:
    monodocs build

# Same, plus rustdoc for the whole workspace under dist/docs/api: each crate's
# modules, structs, enums and functions are then listed in the document itself
# and linked into the generated reference.
[group('docs')]
docs-html-api:
    monodocs build --cargo-doc

# Render and open it
[group('docs')]
docs-open: docs-html
    open dist/docs/index.html

# What monodocs discovered: project, language, kind, path, documents
[group('docs')]
docs-list:
    monodocs list

# Dead links/anchors, unhighlightable code fences, projects with no README.
# `just docs-lint --fix` also applies monodocs' formatting subset first.
[group('docs')]
docs-lint *args:
    monodocs lint {{ args }}

# Markdown normalisation is rumdl's (.rumdl.toml), not monodocs': two formatters
# over one file fight, and rumdl's 84 rules are a superset of the five
# `monodocs fmt` implements. monodocs keeps the checks rumdl cannot make —
# `docs-lint` above resolves links/anchors against the rendered document, knows
# which fence languages the renderer can highlight, and knows which discovered
# projects have no README at all.
# Normalise every markdown file (rumdl)
[group('quality')]
fmt-docs:
    rumdl fmt .

# `rumdl fmt --check`, NOT `rumdl check`: the formatter normalises blank lines
# around headings, fences and lists whether or not the matching lint rule is
# enabled, so `check` alone would pass on files `fmt` would still rewrite.
# Markdown formatting gate — fails when `fmt-docs` would change a file
[group('quality')]
fmt-check-docs:
    rumdl fmt --check .
