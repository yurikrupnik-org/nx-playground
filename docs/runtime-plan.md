# Zed Runtime — Unified Platform Language & Engine

A Rust-native runtime with a clean, declarative language that is both human-readable
and machine-parseable. One language to describe, validate, and execute infrastructure,
services, data pipelines, and deployments across all `really-private1` repos.

## The Problem

Today, the platform uses **7+ languages/formats** to describe the same system:

| Concern | Current Format | Where |
|---------|---------------|-------|
| Service config | TOML (Cargo.toml) | nx-playground |
| API contracts | Protobuf (.proto) | manifests/grpc/ |
| Infrastructure | KCL (.k) | kcl-packages |
| K8s manifests | YAML (kustomize) | apps/*/k8s/ |
| Event routing | YAML (Dapr) | manifests/dapr/ |
| Data pipelines | Rust code (Polars) | libs/analytics/ |
| Orchestration | Nushell (.nu) | scripts/nu/ |
| Service mapping | NUON | manifests/infra-map.nuon |
| Compose services | YAML | manifests/dockers/compose.yaml |
| Quality rules | Rust code | (planned, libs/analytics/quality/) |
| Auth config | Env vars | .env files |

**Consequences:**
- No single view of the platform
- No cross-concern validation (a pipeline references a dataset that doesn't exist? No error until runtime)
- Can't answer "what happens if I remove postgres?" without reading 15 files
- Onboarding requires learning 7 formats
- No reuse across concerns (a "postgres connection" is defined differently in compose, kustomize, Dapr, and Rust)

## The Vision

```zed
# One file describes a complete service + its dependencies + its data

service "zerg-api" {
  runtime = "rust"
  port    = 8080

  depends_on = [postgres, redis, nats]

  route "/api/tasks" {
    handler = "domain_tasks::handlers"
    auth    = jwt
    methods = [GET, POST]
  }

  route "/api/analytics/catalog" {
    handler = "analytics::catalog_handler"
    methods = [GET]
  }
}

connector "postgres" {
  type = database
  image = "postgres:17"
  port  = 5432
  env {
    POSTGRES_DB       = "mydatabase"
    POSTGRES_USER     = "myuser"
    POSTGRES_PASSWORD = secret("pg-password")
  }
  helm = "oci://registry-1.docker.io/bitnamicharts/postgresql"
}

pipeline "revenue_by_category" {
  source   = connector.postgres.query("SELECT * FROM orders WHERE status = 'completed'")
  transform {
    add_column "revenue" = col("quantity") * col("unit_price")
    group_by "category" { sum("revenue"), count("order_id") }
    sort "revenue" desc
  }
  sink = connector.bigquery.table("analytics.daily_revenue")
  schedule = "0 6 * * *"
  quality {
    not_null ["order_id", "revenue"]
    unique "order_id"
    freshness "created_at" < 24h
  }
}
```

**This is not YAML. Not JSON. Not HCL.** It's a purpose-built language that:
- Reads like English
- Validates like a compiler
- Executes like a runtime

## Language Design

### Design Principles

1. **Familiar** — Looks like a mix of HCL + Rust + SQL. No learning curve cliff.
2. **Typed** — Every value has a known type. Catch `port = "8080"` vs `port = 8080` at parse time.
3. **Connected** — References between blocks are first-class. `depends_on = [postgres]` is a typed reference, not a string.
4. **Composable** — Import and extend. A base `connector.postgres` can be specialized per environment.
5. **Executable** — Not just config. Pipelines run, quality checks execute, deployments happen.
6. **Bidirectional** — Can **import** existing YAML/KCL/Proto and **export** to them.

### Syntax Reference

```
# ─── Primitives ──────────────────────────────────────────
string    = "hello"
int       = 8080
float     = 3.14
bool      = true
duration  = 24h | 30m | 5s
cron      = "0 6 * * *"
list      = [1, 2, 3]
map       = { key = "value", port = 5432 }

# ─── References ──────────────────────────────────────────
# Typed references to other blocks (validated at parse time)
depends_on = [postgres, redis]          # connector refs
source     = connector.postgres         # dotted path
handler    = "domain_tasks::handlers"   # Rust path (validated against Cargo workspace)

# ─── Secrets ─────────────────────────────────────────────
password = secret("pg-password")                    # from env/vault at runtime
token    = secret("gcp-sm://project/api-token")     # GCP Secret Manager
key      = secret("vault://secret/data/api-key")    # HashiCorp Vault

# ─── Expressions ─────────────────────────────────────────
# Minimal expression language for data transforms
revenue     = col("quantity") * col("unit_price")
is_active   = col("status") == "active"
threshold   = env("ALERT_THRESHOLD") | 100          # default with |

# ─── Blocks ──────────────────────────────────────────────
# Top-level block types:
connector  "name" { ... }   # Database, message broker, storage, warehouse
service    "name" { ... }   # Rust binary / container
pipeline   "name" { ... }   # ETL / reverse-ETL data flow
quality    "name" { ... }   # Data quality ruleset
schedule   "name" { ... }   # Cron job definition
deploy     "name" { ... }   # Deployment target (local, k8s, cloud)
import     "path"           # Import another .zed file

# ─── Environments ────────────────────────────────────────
# Override blocks per environment
env "dev" {
  connector "postgres" {
    port = 5432
    host = "localhost"
  }
}

env "prod" {
  connector "postgres" {
    host = secret("gcp-sm://project/pg-host")
    helm.values { "primary.resources.requests.memory" = "2Gi" }
  }
}
```

### Block Types

#### `connector` — Data Sources & Sinks

```zed
connector "postgres" {
  type      = database
  direction = both          # source | sink | both
  image     = "postgres:17"
  port      = 5432

  env {
    POSTGRES_DB       = "mydatabase"
    POSTGRES_USER     = "myuser"
    POSTGRES_PASSWORD = secret("pg-password")
  }

  # K8s deployment strategy
  helm = "oci://registry-1.docker.io/bitnamicharts/postgresql"
  helm.values {
    "auth.postgresPassword" = secret("pg-password")
    "primary.service.ports.postgresql" = 5432
  }

  # Health check
  health {
    command  = "pg_isready -U myuser"
    interval = 30s
    retries  = 3
  }
}

connector "bigquery" {
  type        = warehouse
  direction   = sink
  project     = "playground-447016"
  credentials = secret("gcp-sm://playground-447016/bq-credentials")
}

connector "nats" {
  type      = messaging
  direction = both
  image     = "nats:2.10-alpine"
  port      = 4222
  config {
    jetstream = true
    monitor   = true
  }
}
```

#### `service` — Rust Binaries & Containers

```zed
service "zerg-api" {
  runtime    = "rust"
  crate      = "apps/zerg/api"
  port       = 8080
  depends_on = [postgres, redis, nats]

  env {
    DATABASE_URL = connector.postgres.url()
    REDIS_URL    = connector.redis.url()
    NATS_URL     = connector.nats.url()
  }

  # Routes defined declaratively
  route "/api/tasks" {
    handler = "domain_tasks::handlers"
    auth    = jwt { secret = secret("jwt-secret") }
    methods = [GET, POST, PUT, DELETE]
    rate_limit = 100/min
  }

  route "/api/analytics/catalog" {
    handler = "analytics::catalog_handler"
    methods = [GET, POST, DELETE]
  }

  # gRPC service
  grpc {
    proto   = "manifests/grpc/proto/apps/v1/tasks.proto"
    port    = 50051
    service = "TasksService"
  }

  # Deployment
  replicas = 2
  resources {
    cpu    = "500m"
    memory = "256Mi"
  }
}

service "matia-web" {
  runtime = "static"
  crate   = "apps/matia/web"
  port    = 3001
  build   = "bun nx build matia-web"
}
```

#### `pipeline` — Data Flows

```zed
pipeline "revenue_by_category" {
  description = "Daily revenue aggregation by product category"

  source = connector.postgres.query("""
    SELECT * FROM orders
    WHERE status = 'completed'
      AND created_at > now() - interval '7 days'
  """)

  transform {
    add_column "revenue" = col("quantity") * col("unit_price")
    filter col("revenue") > 0
    group_by "category" {
      sum("revenue") as "total_revenue"
      count("order_id") as "order_count"
      avg("revenue") as "avg_order_value"
    }
    sort "total_revenue" desc
  }

  # Multiple sinks
  sink catalog("revenue_by_category")                        # register in Matia catalog
  sink connector.bigquery.table("analytics.daily_revenue")   # push to BigQuery
  sink connector.postgres.table("analytics.daily_summary")   # push back to PG

  schedule = "0 6 * * *"
  timeout  = 5m

  quality {
    not_null ["category", "total_revenue"]
    range "total_revenue" 0..1_000_000
    row_count > 0
    freshness "created_at" < 24h
  }

  on_failure {
    alert connector.nats.topic("alerts.pipeline-failure")
    issue severity = critical
  }
}

# Reverse ETL example
pipeline "user_segments_to_mongo" {
  source = catalog("active_users")

  transform {
    filter col("last_active") > now() - 30d
    add_column "segment" = case {
      col("orders") > 10 => "power_user"
      col("orders") > 3  => "active"
      _                   => "casual"
    }
    select ["user_id", "email", "segment", "last_active"]
  }

  sink connector.mongo.collection("user_segments")
  schedule = "*/30 * * * *"
}
```

#### `quality` — Reusable Quality Rulesets

```zed
quality "financial_data" {
  description = "Standard checks for any financial dataset"

  rule not_null {
    columns = ["id", "amount", "created_at"]
    severity = critical
  }

  rule unique {
    column = "id"
    severity = critical
  }

  rule range "amount" {
    min = 0
    max = 1_000_000
    severity = warning
  }

  rule freshness "created_at" {
    max_age = 1h
    severity = warning
  }

  rule schema_drift {
    severity = warning
    on_change = alert
  }
}

# Apply to a dataset
pipeline "daily_orders" {
  source = connector.postgres.query("SELECT * FROM orders")
  quality = financial_data   # Reference the ruleset
  sink catalog("daily_orders")
}
```

#### `deploy` — Deployment Targets

```zed
deploy "local" {
  type = "kind"
  cluster {
    name    = "dev"
    workers = 2
    ingress = true
  }
  connectors = [postgres, redis, mongo, nats, qdrant, influxdb]
  services   = [zerg-api, zerg-tasks, zerg-email, matia-web]
}

deploy "staging" {
  type = "gke"
  project = "playground-447016"
  region  = "me-west1"
  connectors = [postgres, redis, nats]
  services   = [zerg-api, zerg-tasks, matia-web]
  scaling {
    min = 1
    max = 5
    metric = "nats_consumer_lag"
  }
}
```

## Runtime Architecture

```
                          ┌─────────────────────────────────────────────┐
                          │              zed CLI / Runtime               │
                          │                                             │
  *.zed files ──────────▶ │  ┌─────────┐  ┌──────────┐  ┌───────────┐ │
                          │  │ Parser  │─▶│ Resolver │─▶│ Validator │ │
                          │  │ (logos  │  │ (refs,   │  │ (types,   │ │
                          │  │  +chumsky)│  │ imports) │  │ schemas)  │ │
                          │  └─────────┘  └──────────┘  └─────┬─────┘ │
                          │                                    │       │
                          │              ┌─────────────────────▼─────┐ │
                          │              │         IR (Typed AST)    │ │
                          │              └─────────┬─────────────────┘ │
                          │                        │                   │
                          │  ┌─────────────────────┼─────────────────┐ │
                          │  │    Execution Backends (feature-gated) │ │
                          │  │                     │                 │ │
                          │  │  ┌──────────┐ ┌─────▼────┐ ┌───────┐ │ │
                          │  │  │ Generate │ │ Execute  │ │ Query │ │ │
                          │  │  │          │ │          │ │       │ │ │
                          │  │  │ • YAML   │ │ • Deploy │ │ • DAG │ │ │
                          │  │  │ • KCL    │ │ • Run    │ │ • Deps│ │ │
                          │  │  │ • HCL    │ │ • Test   │ │ • Diff│ │ │
                          │  │  │ • Proto  │ │ • Watch  │ │ • Lint│ │ │
                          │  │  │ • Docker │ │ • Migrate│ │       │ │ │
                          │  │  │ • Nu     │ │          │ │       │ │ │
                          │  │  └──────────┘ └──────────┘ └───────┘ │ │
                          │  └──────────────────────────────────────┘ │
                          └─────────────────────────────────────────────┘
```

### Crate Structure

```
libs/zed/
  zed-lang/               ← Core language (zero deps on infra)
    src/
      lexer.rs            # Token definitions (logos)
      parser.rs           # AST parser (chumsky)
      ast.rs              # Untyped AST nodes
      resolver.rs         # Name resolution, import handling
      types.rs            # Type system (connector, service, pipeline, etc.)
      ir.rs               # Typed intermediate representation
      errors.rs           # Rich diagnostics with source spans
      formatter.rs        # `zed fmt` — canonical formatting
    Cargo.toml            # deps: logos, chumsky, ariadne (errors), miette

  zed-eval/               ← Expression evaluator
    src/
      eval.rs             # Evaluate expressions (col(), env(), secret())
      functions.rs        # Built-in functions (now(), sum(), count(), etc.)
      context.rs          # Runtime context (env vars, secrets, time)
    Cargo.toml            # deps: zed-lang, chrono

  zed-gen/                ← Code generators (bidirectional)
    src/
      mod.rs
      yaml.rs             # Generate kustomize, compose, dapr YAML
      kcl.rs              # Generate KCL packages
      proto.rs            # Generate .proto files from service definitions
      dockerfile.rs       # Generate Dockerfiles
      helm.rs             # Generate helm values
      nu.rs               # Generate Nu scripts
      import/
        mod.rs
        from_yaml.rs      # Import existing YAML → .zed
        from_compose.rs   # Import compose.yaml → .zed
        from_proto.rs     # Import .proto → .zed
        from_kcl.rs       # Import .k → .zed
    Cargo.toml            # deps: zed-lang, serde_yaml, tera (templates)

  zed-runtime/            ← Execution engine
    src/
      mod.rs
      deploy.rs           # Deploy connectors + services (helm, kubectl, docker)
      pipeline.rs         # Execute data pipelines (wraps libs/analytics)
      quality.rs          # Run quality checks
      schedule.rs         # Cron scheduling (wraps NATS)
      watch.rs            # File watcher for dev mode
      diff.rs             # Plan changes (like terraform plan)
    Cargo.toml            # deps: zed-lang, zed-eval, analytics, database, messaging

  zed-cli/                ← CLI binary
    src/
      main.rs
      commands/
        init.rs           # `zed init` — scaffold from existing project
        check.rs          # `zed check` — validate without executing
        plan.rs           # `zed plan` — show what would change
        apply.rs          # `zed apply` — execute changes
        fmt.rs            # `zed fmt` — format .zed files
        graph.rs          # `zed graph` — dependency visualization
        import.rs         # `zed import` — convert existing configs
        export.rs         # `zed export` — generate target formats
        dev.rs            # `zed dev` — watch mode (like tilt up)
        query.rs          # `zed query` — ask questions about the platform
    Cargo.toml            # deps: zed-runtime, zed-gen, clap

apps/zed/                 ← Standalone binary (published)
  Cargo.toml              # Thin wrapper around zed-cli
```

### Dependency Graph

```
zed-lang          (zero external deps besides parsing)
  ↑
zed-eval          (expression evaluation)
  ↑
zed-gen           (code generation — YAML, KCL, Proto, Dockerfile)
  ↑
zed-runtime       (execution — wraps analytics, database, messaging)
  ↑
zed-cli           (CLI commands — clap, colored output)
  ↑
apps/zed          (binary entry point)
```

**Key: `zed-lang` has ZERO infrastructure dependencies.** It can parse and validate
.zed files without Docker, Kubernetes, or any database. This means:
- IDE plugins can use `zed-lang` for syntax highlighting + diagnostics
- CI can run `zed check` without a cluster
- Tests are fast and isolated

## CLI Commands

```bash
# ── Authoring ──────────────────────────────────────────
zed init                        # Scaffold platform.zed from existing project
zed init --from-compose manifests/dockers/compose.yaml
zed init --from-proto manifests/grpc/proto/

zed fmt                         # Format all .zed files
zed fmt platform.zed            # Format one file
zed check                       # Validate types, references, schemas
zed check --strict              # + lint warnings

# ── Planning ───────────────────────────────────────────
zed plan                        # Show what would change (like terraform plan)
zed plan --env prod             # Plan for production
zed graph                       # Print dependency DAG
zed graph --filter services     # Only service dependencies
zed graph --dot | dot -Tpng     # Graphviz output

# ── Querying ───────────────────────────────────────────
zed query "what depends on postgres?"
zed query "which pipelines write to bigquery?"
zed query "show connectors --unused"
zed query "diff dev prod"       # Environment differences

# ── Executing ──────────────────────────────────────────
zed apply                       # Deploy everything
zed apply --only connectors     # Just databases
zed apply --only services       # Just apps
zed apply --env dev             # Target environment
zed apply --dry-run             # Preview without executing

zed run pipeline revenue_by_category          # Run one pipeline
zed run quality financial_data on sales       # Run quality checks
zed run schedule                              # Start scheduler

# ── Development ────────────────────────────────────────
zed dev                         # Watch mode — rebuild + redeploy on changes
zed dev --services zerg-api matia-web         # Only specific services

# ── Code Generation ────────────────────────────────────
zed export yaml                 # Generate all K8s manifests
zed export compose              # Generate docker-compose.yaml
zed export helm                 # Generate helm values files
zed export proto                # Generate .proto from service definitions
zed export nu                   # Generate Nu scripts
zed export kcl                  # Generate KCL packages

# ── Import (migration) ────────────────────────────────
zed import compose manifests/dockers/compose.yaml
zed import kustomize apps/zerg/api/k8s/
zed import proto manifests/grpc/proto/
zed import dapr manifests/dapr/
```

## Implementation Phases

### Phase 0: Language Core (`zed-lang`)

**Goal**: Parse .zed files into a typed AST with rich error messages.

**Crate**: `libs/zed/zed-lang/`

```
Deliverables:
- Lexer (logos): all tokens, keywords, operators
- Parser (chumsky): full grammar → AST
- Resolver: imports, cross-block references
- Type checker: validate types, references, expressions
- Formatter: canonical output (like rustfmt)
- Error reporter: source spans, colored diagnostics (ariadne)
- 50+ tests covering all syntax

Dependencies: logos, chumsky, ariadne, miette, serde
```

**Parser technology choice**:
- **logos** for lexing — fastest Rust lexer, derive macro based
- **chumsky** for parsing — combinator parser with excellent error recovery
- **ariadne** for error display — beautiful source-annotated diagnostics

Why not pest/nom/lalrpop?
- pest: PEG grammars are hard to debug, poor error messages
- nom: Too low-level for a full language
- lalrpop: Good but heavier than chumsky, less composable

### Phase 1: Code Generation (`zed-gen`)

**Goal**: Generate all existing config formats from .zed files.

```
Deliverables:
- YAML generator: compose, kustomize, dapr components/subscriptions
- KCL generator: cluster config, keycloak config
- Helm generator: values files from connector blocks
- Dockerfile generator: from service blocks
- Proto generator: from service.grpc blocks
- Import: compose.yaml → .zed, proto → .zed, dapr YAML → .zed

This phase proves the language can REPLACE existing configs
without changing any infrastructure.
```

### Phase 2: Expression Evaluator (`zed-eval`)

**Goal**: Evaluate pipeline transforms, quality rules, and dynamic values.

```
Deliverables:
- Column expressions: col("x") * col("y"), col("x") > 100
- Aggregations: sum(), count(), avg(), min(), max()
- Functions: now(), env(), secret(), case/when
- Duration arithmetic: now() - 24h
- Secret resolution: env vars, GCP Secret Manager, Vault
- Connection string builders: connector.postgres.url()
```

### Phase 3: Runtime Engine (`zed-runtime`)

**Goal**: Execute .zed files — deploy, run pipelines, check quality.

```
Deliverables:
- Deploy engine: helm install, kubectl apply, docker compose
  (wraps existing Nu infra.nu logic)
- Pipeline engine: translate pipeline blocks → libs/analytics Pipeline API
  (bridges zed transforms to Polars LazyFrame operations)
- Quality engine: translate quality blocks → quality check execution
- Scheduler: cron-based pipeline execution via NATS
- Diff engine: compare current state vs desired state (like terraform plan)
- Watch mode: file watcher → incremental re-deploy
```

### Phase 4: CLI (`zed-cli`)

**Goal**: User-facing binary with all commands.

```
Deliverables:
- All commands listed above
- Shell completions (bash, zsh, fish, nushell)
- REPL mode: `zed repl` for interactive exploration
- LSP server: `zed lsp` for editor integration
```

### Phase 5: Migration & Adoption

**Goal**: Convert all existing configs to .zed, make it the source of truth.

```
Example migration for nx-playground:

Before (7 files):
  manifests/dockers/compose.yaml
  manifests/dapr/components/pubsub-nats.yaml
  manifests/infra-map.nuon
  apps/zerg/api/k8s/kustomize/base/deployment.yaml
  apps/zerg/api/k8s/kustomize/overlays/dev/kustomization.yaml
  manifests/grpc/proto/apps/v1/tasks.proto
  scripts/nu/infra.nu

After (1 file):
  platform.zed
  (all YAML/KCL/Proto generated by `zed export`)
```

## Cross-Repo Integration

### How Each Repo Plugs In

| Repo | Role in Zed | Integration |
|------|-------------|-------------|
| **nx-playground** | Primary platform definition | `platform.zed` lives here, all connectors/services/pipelines defined |
| **kcl-packages** | KCL generation target | `zed export kcl` generates KCL schemas matching existing packages (cluster, keycloak) |
| **yurikrupnik** | Auth service definition | `service "auth-api"` block with WorkOS config, imported into main platform |
| **zerg** | Reference implementation | AI streaming service + TanStack patterns described as service blocks |
| **static-website** | Static deploy target | `service "website" { runtime = "static" }` with S3/GCS deploy |

### Platform File Layout

```
~/really-private1/nx-playground/
  platform.zed              ← Main platform definition
  connectors/
    databases.zed           ← postgres, mongo, redis, qdrant, influxdb
    messaging.zed           ← nats, dapr
    warehouses.zed          ← bigquery, bigtable, s3
  services/
    zerg.zed                ← zerg-api, zerg-tasks, zerg-email, db-worker
    matia.zed               ← matia-web, matia-api (future)
    auth.zed                ← auth service (from yurikrupnik repo)
  pipelines/
    analytics.zed           ← revenue, user segments, metrics rollup
    reverse-etl.zed         ← bigquery sync, mongo sync
  quality/
    rules.zed               ← reusable quality rulesets
  deploy/
    local.zed               ← Kind cluster config
    staging.zed             ← GKE staging
    prod.zed                ← GKE production
```

## Language Grammar (EBNF)

```ebnf
program     = { import | block | env_block } ;
import      = "import" string_lit ;
block       = block_type string_lit "{" { attribute | nested_block } "}" ;
env_block   = "env" string_lit "{" { block } "}" ;

block_type  = "connector" | "service" | "pipeline" | "quality"
            | "schedule" | "deploy" | "route" | "transform"
            | "grpc" | "health" | "scaling" | "resources"
            | "env" | "config" | "rule" | "sink" ;

attribute   = ident "=" expr ;
nested_block = block_type [ string_lit ] "{" { attribute | nested_block } "}" ;

expr        = literal | reference | function_call | binary_expr | list | map ;
literal     = string_lit | int_lit | float_lit | bool_lit | duration_lit | cron_lit ;
reference   = ident { "." ident } ;
function_call = ident "(" [ expr { "," expr } ] ")" ;
binary_expr = expr operator expr ;
operator    = "+" | "-" | "*" | "/" | "==" | "!=" | ">" | "<" | ">=" | "<="
            | "&&" | "||" | "|" ;

list        = "[" [ expr { "," expr } ] "]" ;
map         = "{" [ ident "=" expr { "," ident "=" expr } ] "}" ;

string_lit  = '"' { char } '"' | '"""' { char } '"""' ;
duration_lit = int_lit ("s" | "m" | "h" | "d") ;
cron_lit    = '"' cron_expr '"' ;
```

## Why Build This vs Use Existing Tools?

| Alternative | Why Not Sufficient |
|-------------|-------------------|
| **Terraform/HCL** | Infrastructure only. No data pipelines, no quality rules, no service routing. |
| **Pulumi** | Requires writing Go/Python/TS. Not a clean declarative language. |
| **CUE** | Powerful but steep learning curve. No execution runtime. Config-only. |
| **KCL** | K8s-focused. Can't express pipelines, quality rules, or service routing. |
| **Dagger** | CI/CD focused. No data platform, no K8s deployment. |
| **Pkl** | Apple's config language. No execution, no cross-concern validation. |
| **Nickel** | Functional config. Too academic, no runtime. |

**Zed is unique because it's:**
1. **Full-spectrum** — infra + services + data + quality in one language
2. **Executable** — not just config, it's a runtime
3. **Rust-native** — compiles to the same binaries you already ship
4. **Bidirectional** — imports AND exports existing formats (migration path)
5. **Purpose-built** — designed for your exact stack (Polars, NATS, Dapr, K8s)

## Key Design Decisions

### 1. Logos + Chumsky over Pest/LALRPOP

Pest generates a parser from a grammar file — convenient but produces poor error messages
and is hard to extend. Chumsky is a combinator library that gives us full control over
error recovery, partial parsing, and incremental updates (important for LSP).

### 2. Typed IR over String Templates

The resolver produces a fully typed IR where every reference is resolved and every type
is checked. This means `zed check` catches:
- `depends_on = [postgrs]` → "unknown connector 'postgrs', did you mean 'postgres'?"
- `port = "8080"` → "expected int, got string"
- `sink connector.postgres.table("missing")` → "connector 'postgres' has no 'table' method (it's type 'database', use .query() or .table())"

### 3. Feature-Gated Backends

`zed-runtime` uses Cargo features so you only compile what you need:
- `feature = "helm"` → helm deploy backend
- `feature = "polars"` → pipeline execution
- `feature = "nats"` → scheduler
- `feature = "gcp"` → BigQuery/Bigtable sinks

The `zed check` command works with ZERO features (pure validation).

### 4. Export-First Strategy

Phase 1 generates existing formats. This means:
- Zero risk adoption — keep using kustomize/helm/compose, just generate from .zed
- Gradual migration — convert one concern at a time
- Escape hatch — if Zed doesn't support something, drop to raw YAML

### 5. Platform-Aware, Not Platform-Locked

The language itself knows nothing about Kubernetes or Polars. The type system is
extensible via "connector type" registries. Built-in types cover your stack, but new
connector types can be added without changing the parser.

## Success Metrics

After full adoption:

| Metric | Before | After |
|--------|--------|-------|
| Files to understand the platform | ~50 (YAML + KCL + Proto + Nu + TOML) | 10-15 .zed files |
| "What depends on postgres?" | grep across 15 files | `zed query "depends on postgres?"` |
| Add a new database | Edit 5+ files (compose, infra-map, kustomize, dapr, env) | Add 1 connector block |
| Catch config typo | Runtime failure in K8s | `zed check` at commit time |
| Onboard new developer | Learn 7 formats | Learn 1 language |
| Deploy to new env | Copy + modify 20 YAML files | `zed apply --env staging` |

## Timeline Estimate

| Phase | Scope | Depends On |
|-------|-------|------------|
| **0** | Language core (lexer, parser, type checker, formatter) | Nothing |
| **1** | Code generators (YAML, KCL, Proto, Compose) + importers | Phase 0 |
| **2** | Expression evaluator (pipeline transforms, quality rules) | Phase 0 |
| **3** | Runtime engine (deploy, pipeline exec, quality checks) | Phase 1 + 2 |
| **4** | CLI + LSP + shell completions | Phase 3 |
| **5** | Full migration of nx-playground configs | Phase 4 |
