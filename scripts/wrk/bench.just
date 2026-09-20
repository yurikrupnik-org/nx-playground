# wrk benchmarks against tasks API — imported by the root justfile.

wrk_dir := "scripts/wrk"

api_url_local := "http://localhost:8080/api"

api_url_cluster := "http://localhost:5221/api"

# Benchmark GET /api/tasks (gRPC endpoint) - Local
[group('bench')]
bench-tasks-grpc:
    @echo "=== Benchmarking gRPC Tasks Endpoint (GET) - Local ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/report.lua {{ api_url_local }}/tasks

# Benchmark POST /api/tasks (gRPC endpoint) - Local
[group('bench')]
bench-tasks-grpc-post:
    @echo "=== Benchmarking gRPC Tasks Endpoint (POST) - Local ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/post-task.lua {{ api_url_local }}/tasks

# Run all local tasks benchmarks (GET + POST)
[group('bench')]
bench-tasks-all:
    @echo "======================================"
    @echo "  Tasks API Benchmarks (Local)"
    @echo "======================================"
    @echo ""
    just bench-tasks-grpc
    @echo ""
    just bench-tasks-grpc-post

# Benchmark GET /api/tasks (gRPC endpoint) - Cluster
[group('bench')]
bench-cluster-tasks-grpc:
    @echo "=== Benchmarking gRPC Tasks Endpoint (GET) - Cluster ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/report.lua {{ api_url_cluster }}/tasks

# Benchmark POST /api/tasks (gRPC endpoint) - Cluster
[group('bench')]
bench-cluster-tasks-grpc-post:
    @echo "=== Benchmarking gRPC Tasks Endpoint (POST) - Cluster ==="
    wrk -t4 -c50 -d30s --latency -s {{ wrk_dir }}/post-task.lua {{ api_url_cluster }}/tasks

# Run all cluster tasks benchmarks (GET + POST)
[group('bench')]
bench-cluster-all:
    @echo "======================================"
    @echo "  Tasks API Benchmarks (Cluster)"
    @echo "======================================"
    @echo ""
    just bench-cluster-tasks-grpc
    @echo ""
    just bench-cluster-tasks-grpc-post

# Quick cluster benchmark (10s duration, lighter load)
[group('bench')]
bench-cluster-quick:
    @echo "=== Quick Benchmark: gRPC GET (Cluster) ==="
    wrk -t2 -c10 -d10s --latency {{ api_url_cluster }}/tasks
    @echo ""
    @echo "Benchmark complete!"

# Quick benchmark (10s duration, lighter load) - Local
[group('bench')]
bench-tasks-quick:
    @echo "=== Quick Benchmark: gRPC GET (Local) ==="
    wrk -t2 -c10 -d10s --latency {{ api_url_local }}/tasks

# ============================================================================
# Static web server comparison (nginx vs caddy vs static-web-server)
# Images built from manifests/dockers/Dockerfile targets, same SPA dist.
# ============================================================================

# Behavior-parity tests: health, SPA fallback, content-type, cache headers, gzip
[group('bench')]
test-web-servers dist="apps/todo/web/dist":
    bash {{ wrk_dir }}/web-servers.sh test {{ dist }}

# Benchmark all three servers with wrk; prints req/s + p50/p75/p90/p99 table
[group('bench')]
bench-web-compare dist="apps/todo/web/dist" duration="10s" conns="64" threads="4":
    bash {{ wrk_dir }}/web-servers.sh bench {{ dist }} {{ duration }} {{ conns }} {{ threads }}

# ============================================================================
# Shipped bytes — the repo's ONE byte basis (scripts/bench/).
# Every kB quoted in docs/ or in the stack_profiles table used to be measured
# by hand on a different basis (raw transfer here, gzipped transitive closure
# there), so the numbers could not be compared. These two recipes are that
# basis: assets.sh = what a client downloads, ops.sh = what one operation
# moves.
#
# `bench-assets` with no argument measures the standard arm set —
# apps/todo/web/dist (Solid SPA), apps/todo/web-leptos/dist (Leptos WASM,
# trunk), dist/target/release/x (CLI+TUI) and dist/target/release/todo_web_htmx
# (.cargo/config.toml puts release binaries in dist/target). An arm that is not
# built is reported as `not built` with the command that builds it, never
# silently skipped and never fatal. Arguments override the arm set with any
# dist directory or compiled binary.
# ============================================================================

bench_dir := "scripts/bench"

api_origin_local := trim_end_match(api_url_local, "/api")

# First-render bytes (raw/gzip -9/brotli -q 11) + binary size, cold start, peak RSS
[group('bench')]
bench-assets *arms:
    bash {{ bench_dir }}/assets.sh {{ arms }}

# Bytes on the wire per operation (GET+POST+DELETE /api/todos); needs todo-api running
[group('bench')]
bench-ops base=api_origin_local runs="5":
    bash {{ bench_dir }}/ops.sh --base {{ base }} --runs {{ runs }}
