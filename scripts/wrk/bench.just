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
