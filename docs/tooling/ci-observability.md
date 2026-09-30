# CI / shell observability backend — verdict: trial (2026-09-27, review 2026-12-01)

Question: **how do we observe CI runs, task/shell execution and team outcomes
locally, OSS only** — no SaaS, no account, nothing that needs a card — on the
kind dev cluster, with Postgres `insights` as the analytics store (fixed user
decision, see `docs/ci-insights.md`)?

Concretely the stack has to: chart the four views of the `insights` contract
(CI runs/jobs/steps, task runs/executions, shell scans, scorecard), open a CI
run as a trace, scrape the apps that butler renders with `prometheus = true`,
and give the OTLP endpoint every dev overlay already exports to
(`otel-collector.monitoring.svc.cluster.local:4317`) something to talk to.

Incumbent (the `do nothing` baseline): the GitHub Actions UI per run, the
taskgraph_api web UI (in-memory projection, live facts only), no trace backend
(the dev overlays export OTLP into a Service that does not exist), no shell
static view, no team view. The registry had `opentelemetry-collector` as
`claimed`, `prometheus` as exposition-only, and `[ui.grafana]` /
`[ui.prometheus]` in `butler.toml` pointing at ports nothing served.

## Candidates

| # | candidate | what it is here |
|---|---|---|
| A | do nothing | the incumbent above |
| B | Grafana + Prometheus + Tempo + OpenTelemetry Collector | four Helm releases in namespace `monitoring` (`task observability-up`); Grafana reads Postgres `insights`, Prometheus and Tempo |
| C | Grafana + Prometheus + Jaeger v2 | Jaeger v2 all-in-one (itself an OTel Collector distribution) replaces collector + Tempo; Grafana's Jaeger datasource for traces |
| D | SigNoz (self-hosted) | one OTel-native UI over ClickHouse |

**D fails the must-pass filters and is not scored**: it brings its own
analytics store (ClickHouse) and UI, a second convention next to the decided
Postgres `insights` store — the team/CI SQL would have to be duplicated or
exported, which is a rewrite, not an addition.

All of B's charts are the official ones at pinned versions
(`scripts/tasks/observability.yml`): `prometheus-community/prometheus 29.34.0`,
`open-telemetry/opentelemetry-collector 0.173.1`, and — because Grafana Labs
moved its community charts on 2026-01-30 — `grafana-community/grafana 13.2.6`
and `grafana-community/tempo 3.0.0` (the old `grafana/helm-charts` repo stops
at grafana 10.5.15 / tempo 1.24.4 and gets no releases). Licences: Prometheus,
Collector, Jaeger Apache-2.0; Grafana, Tempo AGPL-3.0 (self-hosted, unmodified:
no obligation triggered). Nothing in B or C has a paid tier in use: no Grafana
Cloud, no plugin downloads (`plugins.preinstall_disabled`), usage reporting off.

## Measurements (kind, context `kind-kind`, one control-plane node, k8s v1.37.0, arm64)

No metrics-server is installed (`kubectl get --raw /apis/metrics.k8s.io/v1beta1`
→ `NotFound`), so per-pod CPU/memory come from the **kubelet Summary API**
(`/api/v1/nodes/<node>/proxy/stats/summary`, instantaneous) cross-checked with
**cAdvisor through the new Prometheus** (5-minute rate), and node totals from
`docker stats` on the kind node container. Pull bytes are containerd's content
sizes (`ctr -n k8s.io images ls`, compressed, what was actually pulled).

### Baseline — empty cluster (2026-09-27T19:55Z)

```text
$ kubectl get --raw /api/v1/nodes/kind-control-plane/proxy/stats/summary | jq '{node_cpu_milli, node_mem_working_set_mib, pods}'
{ "node_cpu_milli": 131, "node_mem_working_set_mib": 915, "pods": 9 }
$ docker stats --no-stream kind-control-plane
kind-control-plane cpu=9.30% mem=917.3MiB / 19.5GiB
```

### Install wall time (B)

```text
# 1. from scratch, cold images (namespace absent, no image cached)
$ /usr/bin/time -p task observability-up
... deployment "grafana" successfully rolled out
real 55.00
user 13.23
sys 1.28

# 2. idempotent re-run on the live stack (one values change → collector rollout)
$ /usr/bin/time -p task observability-up
real 14.75

# 3. teardown, then from an empty namespace with images cached
$ /usr/bin/time -p env OBS_CONTEXT=kind-kind task observability-down
release "grafana" uninstalled ... namespace "monitoring" deleted
real 44.00
$ /usr/bin/time -p env OBS_CONTEXT=kind-kind task observability-up
real 45.75
```

Most of the warm install is Tempo's 20 s readiness delay plus the rollout
waits; a second `observability-down` on the empty cluster exits 0.

### Image pull bytes (B)

```text
$ ctr -n k8s.io images ls   # new images only, compressed content size
docker.io/grafana/grafana:13.2.2-distroless                     411.5 MiB
quay.io/prometheus/prometheus:v3.15.0                           100.0 MiB
docker.io/otel/opentelemetry-collector-k8s:0.160.0               42.1 MiB
docker.io/grafana/tempo:3.0.3                                    32.7 MiB
quay.io/kiwigrid/k8s-sidecar:2.11.2                              29.5 MiB
quay.io/prometheus-operator/prometheus-config-reloader:v0.94.1   12.7 MiB
                                                         total  628.5 MiB
```

Grafana's `-slim` tag was checked and rejected: it drops `plugins-bundled/`,
i.e. the Postgres, Prometheus and Tempo datasources this stack needs, which
would then have to be downloaded — not offline.

### Steady state per pod (B), ~13 min after install

```text
$ kubectl get --raw /api/v1/nodes/kind-control-plane/proxy/stats/summary | jq (monitoring pods)   # 2026-09-27T20:18:17Z
otel-collector-8b75dc6cc-sm9vb       cpu=1m   mem_ws=33Mi
prometheus-server-6b6ff45cb5-jvcrr   cpu=4m   mem_ws=228Mi
grafana-7f64d8dbc8-6t9bt             cpu=11m  mem_ws=595Mi
tempo-0                              cpu=2m   mem_ws=295Mi
node                                 cpu=133m mem_ws=2506Mi
$ PromQL sum by (pod) (rate(container_cpu_usage_seconds_total{namespace="monitoring",container!=""}[5m])) * 1000
grafana 11.3m   otel-collector 1.1m   prometheus-server 5.6m   tempo 3.1m
$ PromQL sum by (pod) (container_memory_working_set_bytes{namespace="monitoring",container!=""}) / 1048576
grafana 593Mi   otel-collector 33Mi   prometheus-server 229Mi   tempo 295Mi
$ docker stats --no-stream kind-control-plane
kind-control-plane cpu=9.14% mem=2.447GiB / 19.5GiB

# per container, same minute (working set / rss)
grafana grafana               385Mi / 305Mi
grafana grafana-sc-dashboard   73Mi /  70Mi
prometheus-server              213Mi / 195Mi
prometheus configmap-reload     14Mi /  13Mi
tempo                          295Mi / 256Mi
otel-collector                  33Mi /  27Mi

# 90 s later (Tempo had flushed its live store)
monitoring/grafana   cpu=11m mem_ws=458Mi   monitoring/prometheus-server cpu=5m mem_ws=238Mi
monitoring/tempo     cpu=3m  mem_ws=83Mi    monitoring/otel-collector    cpu=1m mem_ws=33Mi
```

Grafana was **OOMKilled at a 512 Mi limit** while a browser rendered the four
dashboards (`lastState.terminated.reason: OOMKilled`, exit 137); the values now
request 384 Mi and limit 1 Gi. Idle CPU for the whole stack is ≈ 20 millicores;
memory ≈ 0.8–1.1 GiB working set, the node total rising 915 → 2506 MiB.

### Jaeger v2 (C), same cluster

```text
$ kubectl -n obs-eval create deployment jaeger --image=jaegertracing/jaeger:2.21.0 && kubectl rollout status …
jaeger 2.21.0 all-in-one: create→Ready 4s
image content size: 48.8 MiB
# idle, 90 s later
obs-eval/jaeger   cpu=1m   mem_ws=16Mi
```

(In-memory storage; `obs-eval` was deleted afterwards. C still needs the same
Grafana and Prometheus as B, so its delta is the collector + Tempo pair:
74.8 MiB pull / ≈ 116–328 MiB working set in B vs 48.8 MiB / 16 MiB in C.)

### Image scan (must-pass filter)

```text
$ trivy image --platform linux/arm64 --scanners vuln --severity CRITICAL,HIGH <image>
grafana/grafana:13.2.2-distroless                             CRITICAL=0 HIGH=102 fixable=102
grafana/tempo:3.0.3                                           CRITICAL=0 HIGH=12  fixable=12
otel/opentelemetry-collector-k8s:0.160.0                      CRITICAL=0 HIGH=0
quay.io/prometheus/prometheus:v3.15.0                         CRITICAL=0 HIGH=0
quay.io/kiwigrid/k8s-sidecar:2.11.2                           CRITICAL=0 HIGH=6   (alpine libuuid)
quay.io/prometheus-operator/prometheus-config-reloader:v0.94.1 CRITICAL=0 HIGH=0
jaegertracing/jaeger:2.21.0 (C)                               HIGH=2 (alpine libssl3/libcrypto3)
```

Grafana's HIGHs sit almost entirely in **bundled datasource plugin binaries**
(grpc, x/net, x/text, Go stdlib in the opentsdb, stackdriver, pyroscope,
zipkin, influxdb, tempo plugins) plus one in the server (apache/thrift); Tempo's
are grpc / x/crypto / thrift / stdlib. 13.2.2 and 3.0.3 are the newest stable
tags on 2026-09-27. **This is a filter the trial does not pass cleanly** and
the reason it stays a kind-only trial: nothing is exposed beyond
`kubectl port-forward` on localhost. Adoption requires a re-scan with fewer
HIGHs or an explicit acceptance.

### It works end to end (B)

```text
# OTLP from the real producer, through the collector port-forward, into Tempo
$ OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317 cargo run -q -p taskgraph_cli -- run --offline --trace-url 'trace_id={trace_id}' hell
run 01a0e47a-e7ca-7778-a696-b9c9016d9804  succeeded  target hell  took 341ms
  trace: trace_id=1a9e24b39ba149f1b6a2b4211f7202f5
$ curl -H 'Accept: application/json' localhost:3200/api/v2/traces/1a9e24b39ba149f1b6a2b4211f7202f5
{"service":"taskgraph_cli","spans":["hell","task hell"]}

# backfill: the insights sync replays CI runs with their ORIGINAL timestamps
$ curl -d '<OTLP/JSON span starting 2026-07-29T20:08:04Z>' localhost:4318/v1/traces   → 200
$ curl localhost:3200/api/v2/traces/0c8bd3cb2d6f0465189505955bdc8c98                  → ["workflow run (60d old)"]
$ POST /api/ds/query {queryType:"traceId", range now-1h}                              → status 200, 1 span

# Grafana provisioning
$ curl localhost:55558/api/health
{"database":"ok","version":"13.2.2"}
$ curl -u admin:admin localhost:55558/api/datasources
insights    grafana-postgresql-datasource  postgres.dbs.svc.cluster.local:5432  (db insights)
prometheus  prometheus                     http://prometheus-server.monitoring.svc.cluster.local
tempo       tempo                          http://tempo.monitoring.svc.cluster.local:3200
$ curl -u admin:admin 'localhost:55558/api/search?type=dash-db'
ci-overview "CI overview" · task-runtime "Task runtime" · shell-static "Shell (static)" · team-scorecard "Team scorecard"   (folder "CI insights")
$ GET /api/datasources/uid/<uid>/health
prometheus OK · tempo OK · insights ERROR "no such host" (this kind cluster has no devkit dbs namespace; see below)

# the chart's default kubernetes-pods job scrapes annotated pods (here: the collector's own :8888)
$ curl localhost:55798/api/v1/targets | jq '.data.activeTargets[] | {job, pod, health}'
{"job":"kubernetes-pods","pod":"otel-collector-8b75dc6cc-qxwqr","health":"up"}
```

The butler-rendered annotations on `manifests/k8s/dev/taskgraph-api.yaml`
(`prometheus.io/scrape: 'true'`, `port: '8080'`, `path: /metrics`) are exactly
what that job's relabelling keeps (`regex: true`) and rewrites.

**Dashboard SQL.** Every Postgres query in the four dashboards (54 panel and
variable queries) was executed **through Grafana's own Postgres datasource**
(`POST /api/ds/query`, so `$__timeFrom()`/`$__timeTo()` and the long→wide
time-series conversion are Grafana's) against a scratch database built from
`manifests/db/insights/schema.sql` and seeded with synthetic runs, jobs, steps,
commits of every kind/attribution, task runs with a flaky task, shell scans and
sync rows — once with every variable at *All* and once with specific values:
108 executions, 0 errors; the same 108 against the compose `insights` database
created by `task insights-db`: 0 errors. A browser render of each dashboard
showed every panel populated, and the `trace_id` link in *Recent failures*
opened the `taskgraph_cli` trace in Explore. The scratch database and the
temporary datasources were dropped afterwards. The live `insights` datasource cannot
connect on this particular cluster because it was created bare (no devkit
`dbs` namespace, so no `postgres.dbs.svc`); on a `task local-up` cluster it
resolves.

**Not measured, and why**: p50/p99 latency delta on a real route — nothing in
this stack is in a request path (OTLP export is asynchronous and already
existed); delta to `task check`/`task verify` — the stack adds no step to
either beyond four registry rows (`task tooling-check`) and two generated
port-forward blocks (`task tilt-check`), and whole-workspace gates were out of
scope for this change.

## Score

Weights from `skill://cncf-manager`. `do nothing` gets full marks where it has
nothing to cost (footprint, exit, licence, health) — the rubric rewards absence,
which is the point of the ≥ 15 band.

| criterion | w | A do nothing | B Grafana+Prom+Tempo+Collector | C Grafana+Prom+Jaeger v2 |
|---|---|---|---|---|
| fit to the stated problem | 25 | 2 — GitHub per-run UI only; no task history, shell view, team view or trace backend | 24 — all four views, CI runs as traces with TraceQL, live `taskgraph_*` metrics, one UI for SQL/PromQL/traces | 21 — same Grafana views; Jaeger datasource has trace-by-id + service/operation search, no TraceQL |
| operational cost | 20 | 18 — nothing to run, but every dev overlay exports OTLP into a missing Service | 12 — four releases from three chart repos, one OOM already found and fixed | 14 — three releases; collector and backend are one process |
| measured footprint | 15 | 15 | 8 — 628.5 MiB pull, ≈ 0.8–1.1 GiB working set, 46–55 s install | 10 — ≈ 602 MiB pull, ≈ 0.7–0.85 GiB |
| project health | 15 | 15 | 12 — Prometheus/OTel graduated; Grafana/Tempo single-vendor, chart repo just moved, 102 + 12 HIGH | 14 — CNCF graduated, 2 HIGH (openssl) |
| exit cost | 10 | 10 | 9 — `task observability-down`; the SQL views and dashboards JSON outlive it | 9 |
| licence + money | 10 | 10 | 9 — AGPL-3.0 (self-hosted, unmodified), no paid tier | 10 — Apache-2.0 |
| repo fit | 5 | 2 — leaves `opentelemetry-collector` claimed and `[ui.grafana]`/`[ui.prometheus]` pointing at nothing | 5 — Helm-by-Taskfile like the Atlas operator, the Service name the overlays already use, ports `butler ui` already declares | 3 — a Jaeger process behind the `otel-collector` name, plus a second trace UI |
| **total** | 100 | **72** | **79** (+7) | **81** (+9) |

## Verdict: `trial` — B, kind namespace `monitoring` only, review 2026-12-01

By the rubric alone neither B nor C clears the +15 band over `do nothing`.
B is taken as a **trial**, not an adoption, because:

1. the stack (Grafana + Prometheus + Tempo + collector on kind) is a **fixed
   human decision** for this feature (`docs/ci-insights.md`, decision 1), and
   the rubric's gap is dominated by footprint/ops points `do nothing` earns by
   not answering the question at all (fit 2/25);
2. the blast radius is one namespace on a local cluster, installed and removed
   by one command each, with every version pinned;
3. C's +2 over B is inside noise and trades TraceQL (search CI runs by span
   attributes such as `ci.job`, `conclusion`, duration) for footprint; the
   trace links the dashboards need work with either.

Registry: `grafana`, `tempo` (new, trial), `prometheus` (trial, now installed
here), `opentelemetry-collector` (claimed → scored candidate → trial),
`jaeger-tempo-grafana` (candidate → rejected, superseded by those rows),
`shellcheck` (new, adopted, gap `observe-only`: it feeds the shell view and
never fails a build).

Dev-only credentials, documented where they live
(`manifests/observability/grafana.values.yaml`): Grafana `admin` / `admin`;
datasource `insights` uses the dev Postgres `myuser` / `mypassword`.

## What would reverse it (at or before the review)

- **→ rejected, `task observability-down`**: by 2026-12-01 the insights sync
  is not running against it, or the team reports it has not used the
  dashboards (OSS Grafana keeps no per-dashboard usage stats, so ask) — the
  stack is then weight without a consumer.
- **→ swap Tempo + collector for Jaeger v2 (C)**: TraceQL goes unused and the
  footprint matters (e.g. the dev cluster moves to a smaller machine).
- **→ adopted**: a kind smoke job applies the stack and waits for Ready (closes
  the `no-cluster-in-ci` gap for these rows), and the Grafana/Tempo images
  re-scan without the plugin-bundled HIGHs or the residual ones are accepted
  in writing.
- **Hard stop**: any step toward Grafana Cloud, a Grafana account, or a paid
  plugin is `paid = true` and goes to a human first.
