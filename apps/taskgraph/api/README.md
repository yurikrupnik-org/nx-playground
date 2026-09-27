# taskgraph_api

Read side of taskgraph: folds the `TASKGRAPH` JetStream stream (published by
the [`taskgraph` CLI](../cli/README.md)) into an in-memory projection and serves
the Taskfile drill-down, run timelines and estimates.

No database: the stream (`EventLog`, 7 days / 100k messages) is the source of
truth. On start the service replays every retained fact through an ordered
ephemeral consumer into a projection built off to the side, swaps it in once
caught up (`/readyz` goes green), then applies live facts in place. A broken
subscription rebuilds from the stream rather than patching.

## Surface (one port, default 8080)

| Route | |
|---|---|
| `GET /` | embedded UI: tasks list, drill-down tree, critical path, used-by, run timeline (Gantt), live event feed |
| `GET /api/graphs` | published Taskfiles (newest first) |
| `GET /api/graphs/{id}` | graph + entry points + per-task stats |
| `GET /api/graphs/{id}/tasks/{task}?depth=6` | drill-down: tree, dependents, closure, stats, estimate, recent executions |
| `GET /api/runs?graph={id}&limit=50` | run summaries (newest first) |
| `GET /api/runs/{run_id}` | executions with parent/via, self time, commands, trace id |
| `GET /api/events/sse` | live facts as named SSE events (`task_finished`, …) |
| `GET /api/config` | UI config (trace link template, readiness) |
| `GET /healthz` · `/readyz` · `/metrics` | liveness · replay caught up · Prometheus |

Metrics (live facts only — replayed history is not re-counted on restart):
`taskgraph_events_total{type}`, `taskgraph_task_executions_total{task,outcome}`,
`taskgraph_task_duration_seconds{task,outcome}`, `taskgraph_runs_total{outcome}`,
`taskgraph_run_duration_seconds{outcome}`, `taskgraph_projection_applied`,
`taskgraph_projection_pending`, `taskgraph_events_undecodable_total`, plus the
`axum_helpers` HTTP RED series.

## Configuration

| Env | Default | |
|---|---|---|
| `NATS_URL` | `nats://localhost:4222` | retried with backoff at start |
| `HOST` / `PORT` | `0.0.0.0` / `8080` | |
| `TASKGRAPH_TRACE_URL` | unset | e.g. `http://localhost:16686/trace/{trace_id}` for run trace links |
| `APP_ENV`, `RUST_LOG`, `OTEL_EXPORTER_OTLP_ENDPOINT` | | as every service (`core_config::tracing`) |

## Run locally

```sh
docker compose -f manifests/dockers/compose.yaml up -d nats
HOST=127.0.0.1 PORT=5262 cargo run -p taskgraph_api
cargo run -p taskgraph_cli -- run <task>     # from any directory with a Taskfile
open http://127.0.0.1:5262
```

In kind: `apps/taskgraph/api/butler.toml` (namespace `taskgraph`, host port 5262,
NATS at `nats.dbs.svc.cluster.local`).
