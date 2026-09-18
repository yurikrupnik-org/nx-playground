# Tiltfile generation — imported by the root justfile.
#
# nx owns the app LIST: tools/nx/tilt-targets.ts infers a `tilt-gen` /
# `tilt-check` target on every project that ships k8s manifests and has a
# recognizable kind, so `nx show projects --with-target tilt-gen` IS the app set.
# It is one of three inference modules behind the repo's single registered nx
# plugin, tools/nx/plugin.ts — tilt-targets.ts, rust-targets.ts (`build`/`run`
# per cargo crate) and container-targets.ts (`container`/`scan` per deployable
# app) — all sharing tools/nx/butler-config.ts to read the root butler.toml.
# butler owns the LOGIC: `docker_build(only=...)` needs the transitive,
# dev-dependency-excluding cargo closure, which nx's graph cannot express.
#
# Config, two levels:
#   butler.toml          repo-wide  (registry, env, image conventions per kind,
#                        infra port-forwards, shared cluster resources)
#   <app>/butler.toml    OPTIONAL   (host port, ordering, non-conventional image)

# Regenerate every Tiltfile: per-app through nx (cached, affected-aware), then
# the root Tiltfile from exactly the app set nx reported.
[group('tilt')]
tilt-gen:
    #!/usr/bin/env bash
    set -euo pipefail
    bun nx run-many -t tilt-gen
    cargo run --quiet -p butler -- tilt gen --root --apps "$(just _tilt-apps)"

# Regenerate one app's Tiltfile (nx-native, cached).
[group('tilt')]
tilt-gen-app project:
    bun nx run {{ project }}:tilt-gen

# Fail if any Tiltfile has drifted from the generator (part of `just verify`).
[group('tilt')]
tilt-check:
    #!/usr/bin/env bash
    set -euo pipefail
    bun nx run-many -t tilt-check
    cargo run --quiet -p butler -- tilt gen --root --check --apps "$(just _tilt-apps)"

# Comma-separated project roots of every nx project with a `tilt-gen` target.
# One graph read, so the list cannot disagree with what `run-many` executed.
[group('tilt')]
_tilt-apps:
    #!/usr/bin/env bash
    set -euo pipefail
    d=$(mktemp -d)
    g="$d/graph.json"
    trap 'rm -rf "$d"' EXIT
    bun nx graph --file="$g" > /dev/null
    jq -r '[.graph.nodes[] | select(.data.targets["tilt-gen"]) | .data.root] | sort | join(",")' "$g"
