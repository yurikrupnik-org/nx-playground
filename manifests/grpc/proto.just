# Proto/gRPC workflow (buf) — imported by the root justfile.

proto_dir := "manifests/grpc"

# Format proto files
[group('proto')]
proto-fmt:
    cd {{ proto_dir }} && buf format -w

# Fail when that formatting would change a file (aggregated by `just fmt-check`)
[group('proto')]
proto-fmt-check:
    cd {{ proto_dir }} && buf format -d --exit-code

# Lint proto files
[group('proto')]
proto-lint:
    cd {{ proto_dir }} && buf lint

# Check for breaking changes (against the main branch).
# NOT `cd {{ proto_dir }} && buf breaking --against '.git#...'`: the `.git` input is
# resolved relative to the cwd, so from inside manifests/grpc it looks for
# manifests/grpc/.git and dies with "does not appear to be a git repository".
# Run from the repo root instead — the module dir is the input, and `subdir` points
# the historical side at the same buf.yaml.
#
# buf resolves a `.git` input by re-fetching from it, so `branch=main` needs a real
# refs/heads/main *in this clone*. actions/checkout fetches refs/heads/* into
# refs/remotes/origin/* and checks out a detached head — no local main — which is
# why CI died with "fatal: couldn't find remote ref main" while the same recipe
# passed on a dev machine. Resolve whichever ref this clone actually has.
[group('proto')]
proto-breaking:
    #!/usr/bin/env bash
    set -euo pipefail
    if git show-ref --verify --quiet refs/heads/main; then
        against='branch=main'
    else
        if ! git show-ref --verify --quiet refs/remotes/origin/main; then
            git fetch --no-tags --depth=1 origin +refs/heads/main:refs/remotes/origin/main
        fi
        against='ref=refs/remotes/origin/main'
    fi
    buf breaking {{ proto_dir }} --against ".git#${against},subdir={{ proto_dir }}"

# Build/validate proto files
[group('proto')]
proto-build:
    cd {{ proto_dir }} && buf build

# Generate Rust code from proto files
[group('proto')]
proto-gen:
    cd {{ proto_dir }} && buf generate

# Verify generated Rust code compiles
[group('proto')]
proto-check:
    cargo check -p rpc

# Full proto workflow: format, lint, build, generate, verify
[group('proto')]
proto: proto-fmt proto-lint proto-build proto-gen proto-check
    @echo "Proto workflow complete"
