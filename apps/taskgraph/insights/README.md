# taskgraph_insights

The insights sync service: pulls what CI produced into the Postgres `insights`
database, where Grafana reads it (design + metric definitions:
[docs/ci-insights.md](../../../docs/ci-insights.md)). All logic lives in
[`domain_insights`](../../../libs/domains/insights/README.md); this binary is
config, the loop, probes and metrics.

Every `INSIGHTS_SYNC_INTERVAL` it runs one cycle; each stage records success or
failure in `sync_state` and the cycle continues past a failing stage:

| Stage | Reads | Writes |
|---|---|---|
| `runs` | GitHub Actions runs (latest attempt + attempt 1 of re-runs), jobs, steps | `ci_run_attempts`, `ci_jobs`, `ci_steps` |
| `artifacts` | `taskgraph-events-*` / `taskgraph-shell-scan*` artifacts of completed runs | events → NATS `TASKGRAPH`; scans → `shell_*` |
| `commits` | default-branch commits since the cursor + every run head sha; stats + files | `commits`, `commit_files`, `contributors` |
| `warehouse` | durable consumer `insights-warehouse` on `TASKGRAPH`, drained to the end | `task_runs`, `task_executions`, `task_commands` |
| `derived` | stored commits | `reverted`, `fixup_followed` |
| `traces` | completed counted run attempts not yet exported | OTLP spans (original timestamps) → collector → Tempo |

A GitHub rate limit (403/429 with `x-ratelimit-remaining: 0` or `retry-after`)
stops the GitHub stages until the reset time (persisted in `sync_state`), the
other stages still run.

## Surface (one port, default 8080)

| Route | |
|---|---|
| `GET /healthz` | liveness |
| `GET /readyz` | 200 once one cycle finished with no failed stage |
| `GET /metrics` | `insights_sync_cycles_total{outcome}`, `insights_sync_duration_seconds`, `insights_items_total{source}`, `insights_last_success_timestamp_seconds{source}` + the `axum_helpers` HTTP series |

`--once` runs a single cycle, prints one line per stage and exits non-zero if
a stage failed (no HTTP server).

## Configuration

| Env | Default | |
|---|---|---|
| `DATABASE_URL` | required | Postgres `insights` (migrations: `manifests/db/insights`) |
| `GITHUB_TOKEN` | required | read access to Actions + contents |
| `GITHUB_REPOSITORY` | `yurikrupnik-org/nx-playground` | `owner/name` |
| `NATS_URL` | `nats://localhost:4222` | unreachable → `artifacts`/`warehouse` fail and retry next cycle |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset | OTLP/gRPC for CI traces (`service.name=github-actions`); unset skips `traces` |
| `INSIGHTS_SYNC_INTERVAL` | `10m` | `90s` / `10m` / `1h` / bare seconds |
| `INSIGHTS_BACKFILL_DAYS` | `90` | first cycle's horizon for runs and commits |
| `INSIGHTS_CI_WORKFLOWS` | `.github/workflows/ci-optimized.yml` | comma-separated workflow paths the scorecard counts |
| `HOST` / `PORT`, `APP_ENV`, `RUST_LOG` | | as every service (`core_config`) |

## Run locally

```sh
docker compose -f manifests/dockers/compose.yaml up -d postgres nats
task insights-db        # create DB `insights` + apply migrations
task insights-sync      # one cycle with `gh auth token` (BACKFILL_DAYS=90)
```

In kind: `apps/taskgraph/insights/butler.toml` (workload `taskgraph-insights`,
namespace `taskgraph`, host port 5263). The Secret it reads is never committed:
`task insights-secret` creates `taskgraph-insights-secrets` (`DATABASE_URL`,
`GITHUB_TOKEN`) from `gh auth token`; `task insights-db TARGET=kind` provisions
the database.
