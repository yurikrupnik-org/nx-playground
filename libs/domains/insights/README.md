# domain_insights

CI / shell observability + developer insights: everything
[`taskgraph_insights`](../../../apps/taskgraph/insights/README.md) does, as a
library. Scope `taskgraph` (`tools/nx/scope-tags.ts`): it reads and feeds the
taskgraph event log.

| Module | |
|---|---|
| `github` | minimal GitHub REST client: token auth, `Link` pagination, rate-limit → `RateLimited { reset_at }`; runs (sliced by 7 days, GitHub caps a filtered listing at 1000), attempts, jobs+steps, artifacts (+ zip download), commits |
| `classify` | commit → author / kind / assistants / attribution via `core_authorship::classify_commit`; Conventional Commits type |
| `ingest` | artifact zip → `TaskgraphEvent` JSONL (published to `TASKGRAPH`) or `ShellScan` (stored); ledgered by artifact id |
| `warehouse` | durable pull consumer `insights-warehouse` on `TASKGRAPH` (explicit ack, unlimited redelivery), drained until nothing is pending |
| `traces` | completed counted run attempts → `SpanData` (run → job → step, task runs → executions) exported over OTLP; trace id = first 16 bytes of sha256(`github:<run_id>:<attempt>`) |
| `store` | every SQL statement; idempotent upserts keyed by natural ids |
| `sync` | one cycle: runs → artifacts → commits → warehouse → derived flags → traces |

## Invariants

- **Re-running never duplicates.** Runs by `(run_id, attempt)`, jobs by id,
  commits by sha, facts by `event_id` (`tg_events` ledger), scans by `scan_id`,
  artifacts by id. Facts of one taskgraph run may arrive in any order.
- **Ack after commit.** A warehouse batch is acked only after its transaction
  commits; an undecodable message is recorded in `sync_errors` and terminated
  (the stream is an event log: that moves only this consumer's cursor).
- **Exported means accepted.** An attempt gets `trace_id`/`trace_exported_at`
  only after the OTLP exporter accepted its batch.

## Database

`manifests/db/insights` (versioned). Grafana reads only the `*_v` views and
`scorecard(from, to, min_commits)` / `kind_scorecard(from, to)` — Contract 4
in [docs/ci-insights.md](../../../docs/ci-insights.md).

## Tests

```sh
cargo test -p domain_insights   # docker: Postgres 18 + NATS testcontainers
```

`store_it` (idempotency), `scorecard_it` (formulas, ranking, team-value
neutrality, `why`), `warehouse_it` (drain against JetStream), `contract_it`
(the Grafana-facing columns), unit tests for pagination, rate limits, artifact
decoding and span trees.
