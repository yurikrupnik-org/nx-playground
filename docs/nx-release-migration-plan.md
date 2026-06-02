# Migrate release workflow to `nx release`

## Goal

Replace the custom bash versioning logic in `.github/workflows/release.yml` with `nx release`, so version bumps follow conventional commits (feat → minor, fix → patch, `!`/`BREAKING CHANGE` → major) instead of always being patch bumps.

## Why

Today every release is a flat patch bump regardless of commit type. A `feat:` and a `fix:` produce the same version delta, and a breaking change can ship silently as a patch. We also maintain ~110 lines of bash that re-implement what `nx release` does natively.

## Scope

In scope:
- All projects with a `container` target (the matrix the current workflow already feeds).
- Independent versioning per project (mirrors today's behavior — no lockstep).
- Per-project changelogs.
- Keep `<project>@<version>` tag format so existing tags remain coherent.

Out of scope (for now):
- Publishing to npm / crates.io. These are private services; "publish" = `docker push`, which the existing container matrix handles.
- Workspace-level changelog.

## Repo realities to design around

- **Mixed Rust + JS.** Rust workspace at root with 22 members (3 apps under `apps/zerg/*`, the rest libs). One JS app: `apps/zerg/web`. `nx release` natively versions `package.json` only.
- **`@monodon/rust@2.3.0`** is installed. Need to confirm whether this version exposes a release/version generator for `Cargo.toml`. If not, write a small custom version actions hook.
- **`Cargo.lock` regeneration** must follow any `Cargo.toml` bump (existing workflow already does this).
- **`[skip ci]` on the release commit** — required to avoid recursive triggers from the push-to-main flow.

## Steps

### 1. Verify `@monodon/rust` release support
- Inspect `node_modules/@monodon/rust` for a `release` / `version` generator and any docs.
- If present: use it. If not: plan a custom version actions module (TS file referenced from `nx.json` `release.version.generator`) that reads/writes `version = "x.y.z"` in `Cargo.toml`.

### 2. Add `release` config to `nx.json`
Sketch (refine after step 1):

```json
"release": {
  "projects": ["apps/*", "apps/zerg/*"],
  "projectsRelationship": "independent",
  "releaseTagPattern": "{projectName}@{version}",
  "version": {
    "conventionalCommits": true,
    "generatorOptions": { "currentVersionResolver": "git-tag" }
  },
  "changelog": {
    "projectChangelogs": true,
    "workspaceChangelog": false
  },
  "git": {
    "commit": true,
    "commitMessage": "chore(release): publish [skip ci]",
    "tag": true
  }
}
```

Filter `projects` to only those with a `container` target if we want to keep parity with today's "no container, no release" rule. Otherwise broaden later.

### 3. Dry-run against current state
- `bun nx release --dry-run --first-release=false`
- Verify each project's "current version" resolves from existing `<project>@<version>` git tag.
- Verify proposed bumps match what conventional-commit history implies.

### 4. Trim the workflow
Replace `release.yml:53-165` (everything from "Determine affected apps" through the version-bump commit/tag/push) with:

```yaml
- run: bun nx release --yes --skip-publish
- name: Capture released projects
  id: released
  run: |
    # parse `git tag --points-at HEAD` to build the matrix for downstream container job
```

Downstream `container` and `github-release` jobs stay — they consume the matrix the same way, just sourced from tags instead of computed inline.

### 5. Cargo.lock + Rust crate publish ordering
- Keep the `cargo generate-lockfile` step, but run it inside the `nx release` flow (either as a preVersion hook target or as a step right after).
- If we ever do publish to crates.io: dependency ordering matters (libs before apps; ~30s indexing delay between publishes). Out of scope now, capture as a follow-up.

### 6. Validate end-to-end on a throwaway branch
- Push a branch with one `feat:` and one `fix:` touching different projects.
- Confirm:
  - `feat:` project gets a minor bump.
  - `fix:` project gets a patch bump.
  - Untouched projects don't get bumped.
  - Tags, changelogs, and the `[skip ci]` release commit all land on main.
  - Container job builds only the released projects.
  - GitHub Releases get created per project.

### 7. Cleanup
- Delete the inline bumping bash from `release.yml`.
- Document the release flow briefly in `README.md` or `docs/`.

## Risks / gotchas

- **First run after migration.** If `currentVersionResolver: git-tag` can't find a tag for some project, it'll error or default to `0.0.0`. Confirm every container-targeted project already has at least one `<name>@<ver>` tag, or seed them manually before flipping the workflow.
- **Conventional commit hygiene.** Today the repo isn't strictly conventional-commits. After migration, sloppy commit messages = wrong bumps. Consider a commit-msg lint hook (lefthook is already in the repo — `lefthook.yml`).
- **Rust version actions custom code.** If we end up writing this, it lives in the repo and becomes ours to maintain. Small, but real.
- **Downstream matrix construction.** The current matrix is built inline; the new one needs to derive from `git tag --points-at HEAD` after `nx release` runs. Slightly more brittle if `nx release` changes its tag-creation timing across versions.

## Done when

- A push to `main` with a `feat:` to one Rust crate and a `fix:` to one JS app produces:
  - `<rust-crate>@x.(y+1).0` tag + GitHub release + container.
  - `<js-app>@x.y.(z+1)` tag + GitHub release + container.
  - Per-project `CHANGELOG.md` files updated.
  - One `chore(release): publish [skip ci]` commit on main.
- The bash-based version computation in `release.yml` is gone.

## Rough effort

Half a day if `@monodon/rust` has a usable version generator. ~1 day if we need to write a custom Cargo version actions module. Plus dry-run iteration time.
