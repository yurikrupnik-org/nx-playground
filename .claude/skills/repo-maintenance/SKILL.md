---
name: repo-maintenance
description: Scheduled/hub maintenance playbook for nx-playground, kcl-packages and dotconfig — drift gates, pin freshness, dependency refresh via upkg, issue/PR triage — delivered as pull requests only. Use when running `.github/workflows/claude-maintenance.yml`, when asked to "do maintenance" on one of the three repos, or when triaging stale issues/PRs.
---

# Repo maintenance

Runs from the nx-playground hub: this repo's skills, hooks and AGENTS.md are the
context; the TARGET repo is checked out under `repos/<name>` (in a local session
the target may be the cwd itself or `~/gitorgs/kcl-packages` / `~/dotconfig`).
`upkg` is on PATH (CI installs it from dotconfig's `config/scripts/upkg.nu`).

## Invariants (the hooks enforce the git ones)

- Never commit on `main`, never push `main`, never force-push, never merge —
  `.claude/hooks/guard-git.js` denies these. One PR per task on
  `maintenance/<task>` (`drift`, `pins`, `deps`, `triage-…`), rebased on
  `origin/main`. If that branch/PR already exists, update it (`git push` to
  the same branch, `gh pr edit --body`) instead of opening a second one.
- A PR needs a green gate for the thing it changes (table below). No gate, no
  PR — report instead. Never "fix" a gate by loosening it.
- Pre-existing failures unrelated to the task are REPORTED, not fixed, unless
  the fix is a one-line mechanical one named in the repo's rules.
- Nothing from `.env*`, `secrets/`, tokens or `printenv` ever enters a diff,
  PR body or comment (`guard-secrets.js`). Conventional Commits.
- `task = all` runs every task below in order; each is independent — a failed
  task does not stop the next.

## Tasks

### drift — regenerate what is generated, PR when stale

| repo | gate (must pass BEFORE and AFTER) | regenerate |
|---|---|---|
| nx-playground | `just tilt-check`, `just k8s-check`, `just container-check`, `just boundaries`, `just gen-ci` (its output is gitignored — the gate is that the KCL CI generator still renders) | `just tilt-gen`, `just k8s-gen` |
| kcl-packages | `just mod-check`, `just fmt-check`, `just check` (nx build/test/lint, providers excluded) | `just fmt` |
| dotconfig | `just generate` exits 0 (`output/` is gitignored — nothing to diff); `nu --ide-check 100 config/scripts/*.nu` prints no `diagnostic`; `just brew-preflight` | none — the generator is the artifact |

Never edit generated trees by hand (`libs/**/types`, `libs/rpc/src/generated`,
`docs/openapi`, every `Tiltfile`, `manifests/k8s/apps/**`, dotconfig `output/`).

### pins — versions that must move by hand

- nx-playground `butler.toml` `[k8s] tag` vs `~/gitorgs/kcl-packages/packages/app/kcl.mod`
  `version` (in CI: `npm view` is useless here — read the checked-out
  kcl-packages or `kcl mod metadata` on `oci://docker.io/yurikrupnik/app`).
  Behind → bump, `just k8s-gen tilt-gen`, gate with `just k8s-check tilt-check`.
  Keep it PINNED.
- `testcontainers = "=0.27…"` unpin condition: `curl -s
  https://index.crates.io/te/st/testcontainers-modules | tail -1` requires
  `^0.28` → unpin per the `deps-maintenance` skill; else leave.
- `osv-scanner.toml` / justfile `audit` ignores: for each RUSTSEC id, if
  `cargo audit` no longer reports it without the ignore, drop it from BOTH.
- kcl-packages: every `composition.yaml` `source: …?tag=` equals its package's
  `kcl.mod` version (`just mod-check` covers name; check the tag too);
  `packages/providers/registry.yaml` images vs the `crossplane-contrib` latest
  patch of the same minor — report, do not bump (provider bumps regenerate
  schemas: out of scope).

### deps — the mandated path only

Read `skill://deps-maintenance` first. nx-playground and kcl-packages:
`just upkg` (safe mode; needs docker for testcontainers — present on
ubuntu-latest). dotconfig: `just outdated` is report-only; no auto-bump.
Handle the `BEGIN-UPKG-DIAGNOSIS` block per that skill. A cross-major bump that
breaks the gate is either fixed against the new API in the same PR or
exact-pinned with the unpin condition in the PR body — never left red. PR
title `chore(deps): weekly refresh`, body = the diagnosis block + per-ecosystem
version table.

### triage — issues and PRs, no code

- New issues (`gh issue list --state open --search "created:>=<7d ago>"`):
  label by scope (`scope:zerg|todo|terran|tasks|shared|platform`), add
  `needs-repro` with ONE concrete question when the report lacks a command or
  version, close obvious duplicates with a link. Never argue; never close a
  non-duplicate.
- PRs idle > 14 days with failing/red CI: one comment naming the failing job
  and the `just` recipe that reproduces it. Idle > 30 days, author is a bot:
  close with a note. Human-authored stale PRs are commented, never closed.
- One summary comment per run on the tracking issue titled
  `Maintenance log` (create it if missing, label `maintenance`).

## Report (final message, also the PR body skeleton)

```text
repo: <slug>   task: <task>   date: <YYYY-MM-DD>
gates before: <recipe → pass|fail (reason)>
changes: <bullets, exact files>
gates after:  <recipe → pass>
PRs: <url per task> | none (why)
skipped/pre-existing: <bullets, with the evidence>
```
