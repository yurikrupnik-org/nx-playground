---
name: precommit-review
description: Two-pass review of a staged change before it is committed — pass 1 by the model that wrote the code, pass 2 by independent components (reviewer + security-reviewer subagents, CodeRabbit CLI, Codex CLI) that never see pass 1. Use before any commit of non-trivial code, when asked to review changes / "review before commit" / run /precommit, or when a change is about to go into a PR.
---

# Two-pass pre-commit review

One change, reviewed twice by parties with different blind spots: **the author
pass** (this model, which knows the intent and therefore forgives itself) and
the **independent pass** (agents and vendor CLIs with no memory of writing it,
which catch what intent hides). Findings are then reconciled against evidence,
not vote count.

Entry points: `/precommit` (this protocol), `/precommit-quick` (skips pass 2 —
legal only when the bundle reports docs/config classes only).

## Invariants

- **Review the staged index, never memory of the edit.** Start with
  `just review-bundle`; it freezes `git diff --cached` plus a file table, a
  class per file and the gates that diff implies into
  `dist/review/bundle.md`. Every component reads that one file. A `Stale`
  section means a staged file is also dirty in the working tree — stage or
  revert before reviewing, or the review is of bytes nobody will commit.
- **Machine gates before models.** Run what the bundle lists. An LLM pass over
  code that does not compile burns both passes and produces noise about
  symptoms of the failure.
- **Pass 2 never sees pass 1.** Anchoring destroys the point of a second
  opinion. Reconcile after, never during.
- **Generated output is not authored code.** The bundle's *Generated output*
  section names the regeneration command and the proof — `just tilt-check`,
  `just k8s-check`, or regenerate-then-`git diff --exit-code` where no drift
  gate exists. Run the proof; do not review those hunks line by line, and
  never hand-fix them.
- **Nothing secret reaches a reviewer.** The bundle hard-stops (exit 1,
  **writing no file at all**) on a secret-shaped staged path — `.env*`,
  `.envrc`, `secrets/`, `*.pem|key|p8|p12|pfx|tfvars`, `id_rsa`, `kubeconfig`,
  `credentials.json`, minus the tracked-on-purpose allowlist (`.env.example`,
  `*.vals.yaml`, `testdata/`) — and on secret MATERIAL in added lines (private
  key blocks, `AKIA…`, `ghp_…`, `sk-…`, inline `token: "…"`). That is a
  narrower net than `.claude/hooks/guard-secrets.js`, which guards the tool
  boundary: neither is a substitute for reading what you staged.
- **The external CLIs see more than the index.** `coderabbit review
  --uncommitted` and `codex exec review --uncommitted` transmit the working
  tree, not `git diff --cached`. The bundle's *Outside the index* section
  lists exactly what that adds; commit, stash or clean it before pass 2, or
  you are sending a third party code nobody reviewed.
- **Never commit on `main`/`master`** — `.claude/hooks/guard-git.js` denies it.
  Conventional Commits are enforced by lefthook's `commit-msg` hook.
- **A paid component is a human decision.** CodeRabbit and Codex bill per use;
  adding a new paid reviewer goes through `skill://cncf-manager` first.

## Pass 0 — machine gates

Run exactly the gates the bundle listed for the classes in the diff, plus any
proof gate for generated output. `just verify` is the superset and is the
pre-push gate; pre-commit runs the subset. Red gate ⇒ fix and re-bundle; do not
start pass 1 on a red tree.

## Pass 1 — author pass (this model)

Re-read the final state of each changed file from the bundle. Answer each of
these in writing; "looks fine" is not an answer:

1. **Contract.** For every changed exported symbol, `lsp references` — is every
   callsite migrated? Are tests and docs that assert the old contract updated?
2. **Cutover.** Any shim, alias, re-export, dead branch or commented-out block
   left behind? The code the change obsoletes is in scope and must be deleted.
3. **Second convention** (AGENTS.md). Does this add a parallel way to do
   something the repo already does — a hand-written manifest beside
   `butler.toml` → KCL generation, a per-crate nx rust task beside
   `just lint-rust`, a new scanner beside `trivy`/`osv-scanner`, a tool with no
   row in `docs/tooling/registry.toml`?
4. **Repo traps** touched by this diff: whole-workspace Rust through nx;
   `biome check --write` used as a gate instead of `biome ci`; a hand-edited
   generated tree; a Dockerfile stage not pinned via `[image]`/
   `[imageDefaults]`/`[dockerfileTarget]`; a new `kube`-linked binary without
   `rustls::crypto::aws_lc_rs::default_provider().install_default()`; an N-API
   addon imported from browser code; `image` declared inside a `[workload]`;
   a `#[ignore]`d test presented as coverage.
5. **Tests.** Does each new test fail on a plausible bug and assert what a
   consumer observes? Delete tests that pin wording, mock echoes, field copies
   or defaults — including pre-existing ones the diff touches.
6. **Cost.** Avoidable allocation/clone/copy in a hot path, N+1 query, blocking
   call in async context, a new dependency where the workspace already has one.

Output: findings as `severity | path:line | claim | evidence`, severity in
`blocker` / `warning` / `nit`.

## Pass 2 — independent components (concurrent, blind)

Precondition: the bundle reports no *Stale* and no *Outside the index* section.
The vendor CLIs review the working tree, so anything dirty or untracked is
transmitted with the change under review.

Launch all available components in ONE batch; they do not talk to each other
and do not receive pass-1 output. Give each the bundle path and tell it to read
`AGENTS.md` first — subagents start blank.

| component | catches | invocation |
|---|---|---|
| `reviewer` subagent | repo-aware quality, contract and convention breaks | `task` with `agent: reviewer` |
| `security-reviewer` subagent | injection, authz, secret handling, unsafe deserialization — evidence-backed | `task` with `agent: security-reviewer` |
| CodeRabbit CLI | line-by-line patterns, cross-file smells, a different training distribution | `coderabbit review --uncommitted --agent -c AGENTS.md` |
| Codex CLI | another vendor model, independent failure modes | `codex exec review --uncommitted "<focus>"` |
| Gemini CLI | optional third vendor | `git diff --cached \| gemini "<review prompt>"` |

Availability is verified, not assumed: as of 2026-09-19 `coderabbit doctor`
reports signed in, `codex login status` reports logged in, and `gemini` is
installed but **not authenticated** (it prompts for an OAuth code and hangs a
non-interactive run — treat as unavailable until someone signs in). A
component that is missing, unauthenticated or slower than ~3 minutes is
cancelled and reported as degraded; it is never silently dropped.

**Minimum bar: two independent components must actually run**, at least one of
them a subagent. Below that, say the review is degraded and why — do not
present a one-component pass as a two-pass review.

## Pass 3 — reconcile

- Merge findings by `(path, line, claim)`. A claim raised by **two or more
  independent components is blocker-eligible regardless of the severity each
  assigned it** — independent agreement is the strongest signal available here.
- A single-source finding is triaged on its own evidence: read the code, run
  the thing. Never resolve a disagreement by counting votes or by deferring to
  the most confident wording.
- **Known false positives in this repo** — reject with the reason, do not
  "fix": the inert `istio-injection` labels and commented-out Istio CRs in
  `manifests/k8s/base/namespace.yaml` (byte-mirrored from gitops-v1);
  `testcontainers = "=0.27.3"` (exact pin is deliberate);
  `CARGO=cargo` prefixes in `libs/native/*/package.json`; the empty
  `SOCKET_CLI_API_TOKEN=` in `.env`; `outputs: []` on inferred crate
  `lint`/`test` targets; Rust `live_update` "missing" from Tiltfiles;
  `--no-tests=pass` on per-crate nextest.
- Severity policy: **blocker** = wrong behaviour, security, data loss, an API
  change with unmigrated callsites, a staged secret, generated drift, a red
  gate, or a test that cannot fail. **warning** = maintainability with a named
  cost. **nit** = drop it if a formatter owns it.

## Pass 4 — fix, then re-review only the delta

Fix every blocker, re-stage, re-run `just review-bundle` and the gates, and run
**the author pass again over the new diff only**. Re-run pass 2 only if the fix
touched logic a reviewer flagged, or added ≥1 new file. Maximum two fix rounds;
after that, stop and report what is unresolved — a third round means the change
needs redesign, not another review.

## Commit

Conventional Commits (`feat|fix|docs|style|refactor|test|chore|build|ci|perf|revert`),
scope = the vertical or lib. The body names what review changed, so the next
reader knows the diff was reviewed and why it looks the way it does. Never
`--no-verify`; lefthook's pre-commit (biome, rustfmt, clippy) is the last
deterministic net.

## Report

```text
bundle:     dist/review/bundle.md — <N> files, +<a>/-<d>, classes <…>
gates:      <recipe → pass|fail> (+ proof gates for generated output)
pass 1:     <n blockers, n warnings, n nits>
pass 2:     reviewer <n> | security-reviewer <n> | coderabbit <n|unavailable> | codex <n|unavailable> | gemini <n|unavailable>
confirmed:  <findings raised by ≥2 components>
fixed:      <bullets, path:line>
rejected:   <finding → evidence for rejecting it>
degraded:   none | <component → why>
commit:     <sha> | ready: <conventional message>
```
