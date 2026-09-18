# Container/scan target drift gate — imported by the root justfile.
#
# The `container` / `scan` targets are not written by hand anywhere: they are
# INFERRED by tools/nx/container-targets.ts from the root butler.toml (registry,
# `[imageDefaults.<kind>]`, `[container] extra`) plus each app's optional
# `[image]` override.
# butler resolves the same facts independently, in Rust, when it renders
# Tiltfiles — two implementations of one truth, in two languages.
# This recipe is what stops them from drifting: it dumps the nx graph and has
# butler diff its own resolution against every inferred target.
[group('quality')]
container-check:
    #!/usr/bin/env bash
    set -euo pipefail
    d=$(mktemp -d)
    g="$d/graph.json"
    trap 'rm -rf "$d"' EXIT
    bun nx graph --file="$g" > /dev/null
    cargo run --quiet -p butler -- container verify --graph "$g"
