# Proto/gRPC workflow (buf) — imported by the root justfile.

proto_dir := "manifests/grpc"

# Format proto files
[group('proto')]
proto-fmt:
    cd {{ proto_dir }} && buf format -w

# Lint proto files
[group('proto')]
proto-lint:
    cd {{ proto_dir }} && buf lint

# Check for breaking changes (against the git main branch).
# NOT `cd {{ proto_dir }} && buf breaking --against '.git#...'`: the `.git` input is
# resolved relative to the cwd, so from inside manifests/grpc it looks for
# manifests/grpc/.git and dies with "does not appear to be a git repository".
# Run from the repo root instead — the module dir is the input, and `subdir` points
# the historical side at the same buf.yaml.
[group('proto')]
proto-breaking:
    buf breaking {{ proto_dir }} --against '.git#branch=main,subdir={{ proto_dir }}'

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
