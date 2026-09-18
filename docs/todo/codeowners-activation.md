# CODEOWNERS activation — when a second user joins (TODO)

- **Status:** Deferred — nothing to do while the repo has one human
- **Date:** 2026-08-31
- **Related:** `.github/CODEOWNERS`, `tools/nx/scope-tags.ts`, `just boundaries`,
  backlog 5.6 / 1.3 in [`../architecture-backlog.md`](../architecture-backlog.md)

## Current state (why this file exists)

`.github/CODEOWNERS` maps every vertical to `@yurikrupnik`. GitHub auto-requests
reviews from matching owners on every non-draft PR — but **never from the PR's own
author**, so with one human the file is inert by construction. The path rows already
mirror the `scope:` tag map enforced by `just boundaries`, so activating team review
later is an owner-handle edit, not a restructuring.

Two facts to remember about the mechanism, both easy to get wrong:

- **Only the LAST matching pattern wins** — owners are not unioned across rows. The
  catch-all `*` row must stay FIRST in the file, or it swallows every vertical row.
- **Invalid owners are silently ignored at PR time.** A handle without write access,
  or a team without explicit repo access, produces no request and no error — the only
  diagnostic is the error annotation on the CODEOWNERS file view on github.com.

## Activation checklist

Do these in order the day user #2 gets a vertical.

### 1. Access before ownership

- [ ] Grant the user (or team) **write** access on `yurikrupnik-org/nx-playground`.
      Org membership alone is NOT enough — an owner row without write access is
      silently dropped (see above).
- [ ] Prefer org teams over individuals once there are 2+ people on a vertical:
      `@yurikrupnik-org/<team>` rows enable round-robin / load-balanced reviewer
      assignment via the team's review settings.

### 2. Edit the file

- [ ] Replace the handle on the vertical's rows in `.github/CODEOWNERS`
      (e.g. all three `zerg` rows: `/apps/zerg/`, `/libs/domains/projects/`, …).
      The row structure mirrors `tools/nx/scope-tags.ts` — if a scope moves there,
      move it here in the same commit.
- [ ] Open `.github/CODEOWNERS` on github.com and confirm **zero error
      annotations** (unknown owner / no write access render as inline errors there,
      and nowhere else).
- [ ] Merge to `main` — GitHub reads CODEOWNERS from the PR's **base** branch, so
      the file only takes effect for PRs opened after it lands on main.

### 3. Make it blocking (rung 2)

Auto-request is advisory: a PR merges fine with the request ignored. To make
ownership enforced, add a ruleset (or classic branch protection) on `main`:

- [ ] *Require a pull request before merging*
- [ ] *Require review from Code Owners*
- [ ] Decide required-approval count (1 is enough at two users)
- [ ] Check the interaction with the release flow: `release.yml` pushes version
      commits to `main` directly (`git push origin main --follow-tags`) — a
      require-PR rule blocks that push. Either exempt the bot/deploy key via the
      ruleset's bypass list, or move releases to a tag-triggered flow first (see
      [`ci-release-improvements.md`](./ci-release-improvements.md) §4).

### 4. Verify end-to-end

- [ ] As user #2, open a PR touching only their vertical (e.g. `apps/terran/`):
      the vertical's owner is auto-requested, nobody else.
- [ ] Touch a file in a DIFFERENT vertical in the same PR: that vertical's owner is
      added as a second requested reviewer.
- [ ] With the ruleset on: confirm the merge button stays blocked until the code
      owner approves — request alone must not satisfy it.
- [ ] Draft PRs request nobody until *Ready for review* — expected, not a bug.

## Division of labor with `just boundaries`

CODEOWNERS gates **who approves a change to files in a path**; `just boundaries`
gates **which dependency edges may exist between scopes**. They share one ownership
map but catch different mistakes: a zerg dev editing todo code trips CODEOWNERS,
`zerg_api` importing `domain_tasks` trips the boundary gate. Keep both keyed to
`tools/nx/scope-tags.ts` — when a scope decision changes (e.g. the grandfathered
`domain_cloud_resources → domain_projects` seam gets resolved), update the map, the
gate exception, and the CODEOWNERS rows together.
