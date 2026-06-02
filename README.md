# NX Playground

Nx monorepo with Rust backend services, a React frontend, and Kubernetes-native deployment. Uses Tilt for local dev orchestration and KCL for CI/CD generation.

## Start the app - (one-time)
```sh
just wif-bootstrap local-yk
```

## Start the app - (kind)
```sh
just cluster-up local-yk
```
```sh
just wif-bootstrap paidevo-local      # creates bucket + pool + providers in bootstrap-491220                      
just wif-bootstrap yurikrupnik-local  # creates bucket + pool + providers in yk's project  
```
## Prerequisites

| Tool | Purpose | Install |
|------|---------|---------|
| [Rust](https://rustup.rs/) (stable) | Backend services | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| [Bun](https://bun.sh/) | Package manager, Nx runner | `curl -fsSL https://bun.sh/install \| bash` |
| [Docker](https://docs.docker.com/get-docker/) | Container builds | Desktop or CLI |
| [kubectl](https://kubernetes.io/docs/tasks/tools/) | Kubernetes CLI | `brew install kubectl` |
| [Tilt](https://tilt.dev/) | Local k8s dev environment | `brew install tilt` |
| [just](https://github.com/casey/just) | Command runner | `brew install just` |
| [Atlas](https://atlasgo.io/) | Database schema management | `brew install ariga/tap/atlas` |
| [KCL](https://kcl-lang.io/) | CI/CD config generation | `brew install kcl-lang/tap/kcl` |
| [cargo-nextest](https://nexte.st/) | Fast Rust test runner | `cargo install cargo-nextest` |
| [direnv](https://direnv.net/) | Auto-load env vars | `brew install direnv` |

### Optional

- [sccache](https://github.com/mozilla/sccache) - Shared compilation cache (used in CI with GCS backend)
- [bacon](https://github.com/Canop/bacon) - Background Rust code checker (`just run`)
- [Istio](https://istio.io/) - Service mesh (for gateway / observability)
- [Kind](https://kind.sigs.k8s.io/) / [k3d](https://k3d.io/) - Local Kubernetes cluster

## Quick Start

```bash
# Install JS dependencies
bun install

# Run full quality checks (fmt + lint + test + audit)
just check

# Start local k8s dev environment (requires a running cluster)
tilt up

# Or run services without k8s
just _docker-up        # Start Postgres, Redis, NATS via Docker Compose
cargo run -p api       # Start the API service
just web               # Start the web frontend
```

## Environment Variables

This project uses `direnv` for automatic environment variable loading:

```bash
# Add to your shell (~/.zshrc or ~/.bashrc)
eval "$(direnv hook zsh)"  # or bash

# Copy env template and edit
cp .env.example .env
vim .env

# Allow direnv
direnv allow
```

## Project Structure

```
apps/zerg/
  api/              # REST + gRPC API service (Axum)
  tasks/            # Background task processor
  email-nats/       # NATS-based email worker
  web/              # React frontend (TanStack Router, Vite)
  shared/           # Shared k8s ConfigMaps & Tiltfile

libs/
  core/
    axum-helpers/   # Axum middleware, error handling, extractors
    config/         # Environment configuration
    field-selector/ # Dynamic field selection for queries
    grpc/           # gRPC client utilities
    messaging/      # NATS connection, health, worker patterns
    proc_macros/    # Derive macros (api_resource, sea_orm_resource, selectable_fields)
  database/         # PostgreSQL connection (SeaORM + SQLx)
  domains/
    cloud_resources/  # Cloud resource domain
    projects/         # Projects domain
    tasks/            # Tasks domain
    users/            # Users domain
    vector/           # Vector/embedding domain (Qdrant)
  notifications/
    email/          # Email templates (Handlebars + Lettre)
  rpc/              # Protobuf/gRPC definitions
  testing/
    test-utils/     # Shared test utilities & testcontainers

scripts/kcl/ci/     # KCL-based CI pipeline generator (GitHub Actions + Tekton)

manifests/
  k8s/              # Kustomize base + overlays (dev/prod)
  schemas/          # Database schema (HCL, SQL, DBML) + seed data
  migrations/       # Atlas migration files
```

## Local Development

### With Kubernetes (Tilt)

```bash
tilt up
```

Tilt manages port-forwards automatically:

| Service | Port | Notes |
|---------|------|-------|
| PostgreSQL | `localhost:5432` | User: `myuser` / DB: `mydatabase` |
| Redis | `localhost:6379` | |
| Mailhog | `localhost:8025` | Email testing UI |
| Istio Gateway | `localhost:8080` | API gateway |
| Kiali | `localhost:20001` | Service mesh dashboard |

Tilt also handles schema ConfigMap regeneration, database seeding, and auto-rebuild of all zerg apps on code changes.

### Database Schema (Atlas)

```bash
just schema-validate         # Validate HCL schema
just schema-sql              # Generate SQL from HCL
just schema-apply            # Apply schema to local DB
just migrate-diff <name>     # Generate migration from schema diff
just migrate-apply           # Apply pending migrations
```

## Nx Commands

```bash
bun nx affected -t lint      # Lint affected projects
bun nx affected -t test      # Test affected projects
bun nx affected -t build     # Build affected projects
bun nx graph                 # Visualize dependency graph
```

### KCL CI Package

```bash
bun nx run kcl_ci:lint       # Lint KCL files
bun nx run kcl_ci:test       # Test KCL files
bun nx run kcl_ci:build      # Generate CI pipeline output
```

## Quality Checks

```bash
just check          # Full: fmt + lint + test + audit
just check-quick    # Compile + lint only (no tests)
just fmt            # Format all Rust code
just lint           # Run Clippy
just test           # Run tests with nextest
just audit          # Security audit + cargo deny
just outdated       # Show outdated dependencies
```

## Architecture

The backend follows a **modular monolith** pattern with 4 layers per domain:

```
Models -> Repository -> Service -> Handlers
```

Each domain (users, projects, tasks, cloud_resources) is a self-contained library with clear boundaries, designed for easy extraction to microservices if needed.

## Documentation

### Architecture & Design

- [Modular Monolith Architecture](docs/modular-monolith-architecture.md) - Domain structure, layer responsibilities, and migration path to microservices
- [Code Reuse Patterns](docs/code-reuse-patterns.md) - Generic repositories, service composition, macros, and error mapping
- [Repository Pattern Comparison](docs/repository-comparison.md) - SqlMethods trait extension vs BasePgRepository composition
- [SQLx vs Sea-ORM](docs/sqlx-vs-seaorm.md) - ORM comparison with benchmarks and migration path

### Communication

- [gRPC Guide](docs/grpc.md) - Communication patterns (unary, streaming, bidirectional) with Rust examples
- [gRPC Optimization Results](docs/grpc-optimization-results.md) - 10.3x throughput improvement from binary schema optimizations
- [gRPC Serialization Optimization](docs/grpc-serialization-optimization.md) - Protocol Buffers optimization reducing wire format by 64%
- [Messaging Patterns](docs/messaging-patterns.md) - Decision guide for gRPC vs Redis Streams vs RabbitMQ vs Kafka

### Operations & Testing

- [Testing Guide](docs/TESTING_GUIDE.md) - Testing pyramid, patterns, and CI integration
- [HPA Local Testing](docs/hpa-local-testing.md) - Horizontal Pod Autoscaler setup with Kind cluster
- [Tasks API Improvements](docs/tasks-api-improvements.md) - Proposed optimizations: compression, caching, batch ops, streaming
- [Development Tools](docs/DEVELOPMENT_TOOLS.md) - AI tools, security scanners, linters, and recommended toolchains

## CI/CD

CI runs on GitHub Actions (`.github/workflows/ci-optimized.yml`) with parallel jobs:

| Job | What it does |
|-----|-------------|
| **lint** | `nx affected -t lint` (Clippy, ESLint, KCL lint) |
| **test** | `nx affected -t test` (nextest, Vitest, KCL test) |
| **build** | `nx affected -t build` (cargo, Vite, KCL run) |
| **container** | Docker build + push + Trivy security scan |

Features: Nx Cloud caching, sccache with GCS backend, Workload Identity Federation, SARIF upload to GitHub Security.

The CI pipeline configuration is generated from KCL in `scripts/kcl/ci/`.
