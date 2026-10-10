---
name: cncf-manager
description: Evidence-driven evaluation, adoption, drift-audit and retirement of infrastructure/CNCF-landscape tooling (mesh, gateway, gitops, operators, scanners, SaaS). Human approval is REQUIRED before anything paid. Use when asked "do we actually use X?", when adding/replacing/removing a platform tool or operator, when a tool appears in docs but maybe not in the cluster, or when a change introduces a paid plan, seat, quota or SaaS token.
---

# CNCF / platform tool manager

One registry, one scorecard, one drift gate. A tool is *adopted* only when the
repo can prove it: an install path, a consumer, and a gate that goes red when it
breaks. Prose is not proof — `README.md` claimed Istio + Kiali for months while
the repo contained no control plane, no CRs and no canary (see
*Worked example*).

Applies to anything below the app line: cluster, gitops, networking, operators,
render/config, secrets, messaging, observability, scanners, build/CI, dev-loop,
test infra — CNCF-landscape or not. Not for application libraries (crate/npm
choices go through the normal comparison-doc route, e.g.
`docs/sqlx-vs-seaorm.md`).

## The registry — `docs/tooling/registry.toml`

Single source of truth; hand-written, reviewed like code, and enforced by
`task tooling-check` (`tools/tooling/check-registry.ts`, inside `task verify`).
The row count is whatever the gate prints; do not restate it here.

```toml
[[tool]]
name        = "gateway-api"           # unique key
category    = "networking"            # cluster|gitops|networking|config|database|secrets|messaging|observability|security|build|dev-loop|testing|other
status      = "adopted"               # adopted|trial|candidate|rejected|external|claimed
cncf        = "incubating"            # graduated|incubating|sandbox|not-cncf|unknown
install     = "devkit.toml"           # WHAT INSTALLS IT — recipe, dep, manifest, workflow step
key         = "[[deps]] gateway-api-crds"  # optional grep token pinning the fact inside that file
usage       = ["manifests/k8s/base/gateway/zerg-api.yaml"]  # 1-3 real consumers
gate        = "task k8s-check"        # the check that fails if it breaks; "none" needs a [[gap]]
paid        = false                   # true => `approval` REQUIRED (human, never the agent)
owner       = "scope:infra"           # scope tag, per tools/nx/scope-tags.ts
decision    = "docs/tooling/….md"     # comparison doc; omit for pre-existing rows
review      = "2026-12-01"            # trials and paid tools only
```

**No line numbers.** They rot within a commit and the gate would fail on
unrelated edits; cite the file, and pin the fact with `key` — whose tokens the
gate greps for inside the cited `install` file, so a version pin
(`cnpg-1.24.0.yaml`, `provider-kubernetes:v1.1.0`) cannot go stale silently.
Every path-looking token in `install`/`usage` (globs allowed) must resolve, so
put prose in `note`/parentheses, never in a citation. Avoid a glob so wide it
cannot fail (`libs/**` is satisfied by any file under `libs/`).

A `gate = "none"` on an `adopted` row is a defect *unless* the row is listed in
exactly one declared gap:

```toml
[[gap]]
name       = "no-cluster-in-ci"
applies_to = ["cloudnative-pg", "atlas-operator"]
why        = "nothing in verify or CI starts a cluster"
closes_when = "a kind smoke job applies manifests/k8s and waits for CRs to go Ready"
owner      = "scope:infra"
```

Status semantics, and the only legal transitions:

| status | means | may become |
|---|---|---|
| `candidate` | scored, not wired | `trial`, `rejected` |
| `trial` | wired behind a flag/one app, `review` date set | `adopted`, `rejected` |
| `adopted` | install + usage + gate all present | `rejected` (with removal PR) |
| `rejected` | evaluated and refused; keep the row so it is not re-litigated | `candidate` (only with new data) |
| `external` | real, but owned by another repo/cluster (e.g. the `main-gateway` Gateway and anything Flux applies from `gitops-v1`) — this repo may reference but MUST NOT install it | `adopted` |
| `claimed` | named in prose, zero install, zero usage — always a defect | `candidate`, `rejected`, or deleted with the prose |

`task tooling-check` enforces: unique names, closed enums, every cited path
resolves, `key` tokens present in the cited install file, `adopted` ⇒ non-empty
`usage`, a gate starting with `none` ⇒ declared in exactly one `[[gap]]`,
`paid = true` ⇒ an `approval` record. It never contacts a cluster — a repo-only
gate that always runs beats a cluster gate that never does; runtime evidence
stays class 3 of the drift audit.

**Residual risk the gate cannot close:** `approval` is self-attested TOML. An
agent can type one. It is a record of a human decision, never a substitute for
asking — adding `paid = true` without having asked is a protocol violation the
reviewer, not the gate, has to catch.

## Evaluating a solution (data-driven, not landscape-driven)

1. **State the problem as a measurable question**, with the incumbent named.
   "Do we need a service mesh?" is not a question; "can we get per-route
   retries + mTLS between `zerg-api` and `zerg-tasks` without a sidecar per
   pod?" is.
2. **Candidates ≤ 4, and `do nothing` is always one of them.** The incumbent
   (today: Gateway API HTTPRoutes + in-process tower layers) is the baseline
   every score is relative to.
3. **Measure here, on a real workload**, the way this repo already does it:
   `docs/todo-delivery-options.md` (247 B vs 110 B per todo),
   `docs/todo-state-management.md` (46.2 kB gz, `+16.0`, `+57.1`),
   AGENTS.md's nx-vs-cargo timings (2m13s vs 4m06s vs 10m09s). Star counts,
   blog posts and CNCF maturity are *inputs to one criterion*, never the
   verdict.
   Minimum measurements for a platform tool: install time on a fresh `kind`
   cluster (`task local-up`), steady-state CPU/memory added per node and per
   pod, p50/p99 latency delta on one real route, image pull bytes, and the
   delta to `task check` / `task verify` wall time.
4. **Must-pass filters** (fail any → `rejected`, no score needed):
   OSI licence or an approved commercial one · works offline in `kind` (no
   mandatory SaaS) · no second convention (AGENTS.md: a new tool that duplicates
   `butler.toml` → KCL → generated manifests is a rewrite, not an addition) ·
   attaches to an existing gate (`task check`/`verify`/CI matrix) · clean
   `trivy` + `osv-scanner` on its images · an exit path that is not a rewrite.
5. **Score the survivors** (weights fixed; total 100):

   | criterion | w | scored on |
   |---|---|---|
   | fit to the stated problem | 25 | does it solve it *whole*, without adjacent rewrites |
   | operational cost | 20 | install/upgrade/debug burden, new failure modes, on-call surface |
   | measured footprint | 15 | the step-3 numbers |
   | project health | 15 | CNCF maturity, release cadence, maintainer count, CVE response time, breaking-change history |
   | exit cost | 10 | hours to remove after 6 months |
   | licence + money | 10 | free tier limits, per-seat/per-node price, what breaks unpaid |
   | repo fit | 5 | collapses into existing inference/gates vs adds files to hand-maintain |

   Adopt only at **≥ 15 points over the incumbent**. Inside that band the
   incumbent wins — churn has a cost this table does not price.
6. **Write the decision doc** in `docs/tooling/<question>.md`: question,
   candidates, the measurement commands *and their raw output*, the scored
   table, the verdict, and the condition that would reverse it. Then add/flip
   the registry row in the same PR.
7. **Adopt as `trial` first** whenever the blast radius is cluster-wide: one
   app, one namespace, a `review` date. Cluster-wide day-one adoption needs the
   decision doc to say why a trial is impossible.

## Human in the loop — anything that costs money

**HARD STOP.** The agent MUST NOT, under any circumstance: create an account,
accept terms, start a trial, enter a card, raise a plan/quota/seat count, enable
a paid tier of a free tool, or commit a token. `paid = true` covers price,
seats, metered quota, an API token, or a hard dependency on a hosted control
plane — a "free tier" is still `paid = true`, because the ceiling is the risk.

Procedure: stop at the point of decision and `ask` the user with this table
filled in; proceed only on an explicit yes.

```text
tool:            <name>            vendor: <org>
plan:            <tier>            price:  <$ / unit / period>
free tier:       <exact limits and what happens at the limit>
billing owner:   <who is charged>  token:  <env var name — value NEVER shown>
if unpaid:       <what breaks, and the fallback>
exit:            <hours to remove; data export path>
```

Record the answer in the registry row and nowhere else:

```toml
approval = { by = "<user>", date = "2026-09-19", cap = "$X/mo", review = "2027-03-01" }
```

Already paid/SaaS-bound in this repo — treat as pre-approved, but any *increase*
re-triggers the gate: Nx Cloud (`nx.json` `nxCloudId`), GCP Secret Manager via
`vals` (`manifests/secrets/*.vals.yaml`, `devkit secrets fetch`), Socket CLI
(`SOCKET_CLI_API_TOKEN` — note the empty-value trap in AGENTS.md), Docker Hub
(`docker.io/yurikrupnik/*` images and the pinned KCL package), the GCS sccache
bucket, GitHub Actions minutes. Secrets never enter a diff, PR body or comment
(`.claude/hooks/guard-secrets.js`).

## Drift audit — "do we really use X?"

Four independent evidence classes; run all four before answering.

| # | class | how to check (repo-only; add the cluster check when one is reachable) |
|---|---|---|
| 1 | **install** | `grep` the tool in `Taskfile.yml`, `scripts/tasks/*.yml`, `devkit.toml` `[[deps]]`, `.github/workflows/**`, `platform/**`, `Cargo.toml`, `package.json`. No installer ⇒ nothing put it in a cluster. |
| 2 | **API objects** | its CRDs/CRs/labels in `manifests/**`, `apps/**/k8s/**`, `platform/**`. Commented-out YAML is evidence of *absence*. |
| 3 | **runtime** | `kubectl get ns/<ns>`, `kubectl api-resources --api-group=<group>`, `kubectl get <cr> -A`. Unreachable cluster ⇒ say so; do not upgrade a repo-only conclusion into a cluster claim. |
| 4 | **gate** | does anything go red if it vanishes? A tool no gate covers is unowned regardless of what is installed. |

Verdicts: install+usage+gate → `adopted`. Usage but no install → `external`
(another repo owns it — verify *which*, then stop) or broken. Install but no
usage → dead weight, propose removal. Prose only → **claimed, not adopted**:
fix the prose in the same PR, and add a `rejected`/`candidate` row so the claim
cannot silently come back.

Never "clean up" a file whose stated purpose is to mirror another repo. Two
live examples: `manifests/k8s/base/namespace.yaml` (header: byte-identical to
gitops-v1 `infrastructure/platform/networking-config/base/namespace.yaml` so
Flux's later apply is a no-op — its `istio-injection` labels stay even though
this repo has no mesh) and `devkit.toml`'s gateway-api CRD pin (v1.2.1, pinned
to match what Flux ships).

### Worked example — the Istio claim (2026-09-19)

| class | finding |
|---|---|
| install | none. No `istioctl`, no helm release, no `IstioOperator`, no `istio-system`; `scripts/tasks/k8s.yml` installs CNPG + Atlas only, `devkit.toml` adds gateway-api CRDs only |
| API objects | none live. `manifests/k8s/base/namespace.yaml` holds a commented-out `AuthorizationPolicy` + `DestinationRule`; the two `istio-injection: enabled` labels are inert without a control plane and present only to mirror gitops-v1 |
| runtime | unverifiable — no cluster reachable |
| gate | none |
| canary | none anywhere: no Flagger, no Argo Rollouts, no `weight:` on any `backendRef`, no `VirtualService` |

Verdict: **claimed, not adopted.** North–south traffic is Gateway API
`HTTPRoute`s (`manifests/k8s/base/gateway/`, `apps/*/web/butler.toml`
`[[env.prod.workload.extraManifests]]`) parented to `main-gateway` in the
`gateway` namespace — a Gateway this repo never defines, i.e. `external`,
owned by gitops-v1. East–west is plain ClusterIP + tower middleware. Action
taken: the Istio/Kiali rows were removed from `README.md`, the `istio` row went
in as `rejected` with `docs/tooling/service-mesh.md` as its decision doc, and
the mirrored namespace labels were left alone.

## Deliverable

Every run of this skill ends with a registry diff **or** an explicit "no change,
here is the evidence" — and with `task tooling-check` green either way. Report:

```text
question:   <what was asked>
method:     <evidence classes checked / measurements run, with commands>
findings:   <bullets, each with a path or raw numbers>
verdict:    adopted | trial | candidate | rejected | external | claimed-not-adopted
registry:   <rows added/flipped>  docs: <decision doc>
paid gate:  n/a | asked → approved/declined (<cap>)
follow-ups: <prose fixed, gate added, removal PR>
```
