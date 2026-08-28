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

# Check for breaking changes (against git main branch)
[group('proto')]
proto-breaking:
    cd {{ proto_dir }} && buf breaking --against '.git#branch=main'

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
