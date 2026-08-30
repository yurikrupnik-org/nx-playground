# App manifest generation — imported by the root justfile.
#
# Same split as the Tiltfile track: nx owns the app LIST (tools/nx/k8s-targets.ts
# infers a `k8s-gen` / `k8s-check` target on every app that declares a
# `[workload]`), butler owns the LOGIC — it merges
# `[workloadDefaults.<kind>] < <app> [workload] < [env.<env>.workload]`, injects
# the facts an app must never hand-type (image reference, name, namespace,
# partOf), writes `<app>/k8s/values.yaml`, and renders it through the pinned KCL
# package from the root butler.toml `[k8s]` section.
#
# Outputs, all generated and drift-gated:
#   <app>/k8s/values.yaml            the merged payload the KCL package validates
#   manifests/k8s/apps/<app>.yaml     that app rendered
#   manifests/k8s/apps/kustomization.yaml   the aggregate, one entry per app

# Regenerate every app's values + rendered manifests, then the aggregate
# kustomization from exactly the app set nx reported.
[group('k8s')]
k8s-gen:
    #!/usr/bin/env bash
    set -euo pipefail
    bun nx run-many -t k8s-gen
    cargo run --quiet -p butler -- k8s gen --root --apps "$(just _k8s-apps)"

# Regenerate one app (nx-native, cached).
[group('k8s')]
k8s-gen-app project:
    bun nx run {{ project }}:k8s-gen

# Fail if any values file, rendered manifest or the aggregate has drifted from
# butler.toml. Part of `just verify`.
[group('k8s')]
k8s-check:
    #!/usr/bin/env bash
    set -euo pipefail
    bun nx run-many -t k8s-check
    cargo run --quiet -p butler -- k8s gen --root --check --apps "$(just _k8s-apps)"

# Comma-separated project roots of every nx project with a `k8s-gen` target.
# One graph read, so the list cannot disagree with what `run-many` executed.
[group('k8s')]
_k8s-apps:
    #!/usr/bin/env bash
    set -euo pipefail
    d=$(mktemp -d)
    g="$d/graph.json"
    trap 'rm -rf "$d"' EXIT
    bun nx graph --file="$g" > /dev/null
    jq -r '[.graph.nodes[] | select(.data.targets["k8s-gen"]) | .data.root] | sort | join(",")' "$g"
