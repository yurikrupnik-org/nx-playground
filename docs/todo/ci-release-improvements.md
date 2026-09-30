# CI / Release Flow Improvements (TODO)

Review of `.github/workflows/ci-optimized.yml` and `.github/workflows/release.yml`
(2026-07-24). Items are prioritized; file:line references point at the code as of
the review. Related infra: CI is generated from `manifests/ci/ci-config.yaml` via
`scripts/kcl/ci` (`kcl run scripts/kcl/ci/main.k -S githubWorkflow`).

## 1. Correctness / safety (do first)

- [x] **Duplicate CI running** — resolved: `generated-ci.yaml` no longer exists in
  `.github/workflows/` (only `ci-optimized.yml`, `release.yml`, `publish-crates.yml`
  and the claude workflows remain, verified 2026-08-31). The follow-up — folding the
  container job into the KCL generator so the generated workflow can replace
  `ci-optimized.yml` — moves to §4.
- [ ] **Release race** (`release.yml:13-15,165`): concurrency group is
  `release-${{ github.sha }}` — per-SHA, so two quick merges run two releases
  concurrently; both `git push origin main --follow-tags` → tag/push collisions.
  Change to `group: release`, `cancel-in-progress: false` (serialize releases).
- [ ] **`cargo generate-lockfile` re-resolves the whole lock** (`release.yml:136`)
  → silently bumps transitive deps inside a release commit. Use
  `cargo update --workspace` instead.
- [ ] **`continue-on-error: true` on container build+scan**
  (`ci-optimized.yml:315`, `release.yml:204`): a failed image build/push does not
  fail the release — a GitHub release can be cut whose image never reached the
  registry. Split build/push (must fail) from scan upload (may soft-fail).

## 2. Reproducibility / supply chain

- [ ] **`bun-version: latest` + `bun install --no-cache`** in every job (5x):
  non-reproducible toolchain and deliberately uncached installs. Pin bun
  (`ci-config.yaml` has `bun_version`), drop `--no-cache`.
- [ ] **Unpinned `curl | bash` installers**: KCL CLI (3x) and Trivy (from `main`
  branch, 2x, with sudo). Use `kcl-lang/setup-kcl@v1` and
  `aquasecurity/trivy-action`, or pin versions/checksums.
- [x] **`rustup toolchain install stable`** moving target — `rust-toolchain.toml`
  exists at the repo root (verified 2026-08-31); repo + CI pin the same toolchain.
- [ ] **SA key vs WIF split-brain**: lint/test/build use
  `credentials_json: GCP_SA_KEY` (long-lived secret) while container jobs use
  keyless WIF. Move sccache auth to WIF, delete the key.
- [ ] **Image signing/SBOM are near-free**: `id-token: write` already granted —
  add cosign keyless signing and `provenance`/`sbom` on the container builds.
- [ ] **SARIF via hand-rolled `gh api` + gzip/base64** (2x, ~40 lines): replace
  with `github/codeql-action/upload-sarif@v3`.

## 3. Speed / cost

- [ ] **Setup x3 duplication**: lint/test/build repeat the identical 9-step setup
  (checkout → shas → rustup → GCP → sccache → bun → install → KCL). Either one
  job running `bun nx affected -t lint test build` (nx schedules internally; pay
  setup once) or a shared composite action emitted by the KCL generator.
- [ ] **No `concurrency` on CI**: add `group: ci-${{ github.ref }}`,
  `cancel-in-progress: true` so force-pushes cancel stale runs.
- [ ] **No `timeout-minutes` on any job** — hung jobs run 6h (workspace tests
  recently hung on container-based tests).
- [ ] `fetch-depth: 0` full clones everywhere — fine at current size; revisit
  with `filter: tree:0` when clone time matters.
- [ ] Delete ~80 lines of commented-out free-disk-space/cleanup blocks and
  `df -h` debugging leftovers.

## 4. Structural

- [ ] **Single source for action pins**: `release.yml`/`ci-optimized.yml` use
  checkout@v7 / nx-set-shas@v5 / sccache@v0.0.10; the KCL registry
  (`scripts/kcl/ci/schemas/common.k GHA_ACTIONS`) pins v4/v4/v0.0.9. Bump the
  registry to the versions the live CI already proved, then generate all
  workflow YAML from `manifests/ci/ci-config.yaml` so pins live in one place.
- [ ] **Generate the release workflow too**: add a `release:` section to
  `manifests/ci/ci-config.yaml` (container_target, scan_target, bump policy,
  tag_format, registry/auth flavor, github_release, sarif_upload) and a
  `generate_monorepo_release` in the KCL package; retire the handwritten file.
- [ ] **Replace hand-rolled versioning with `nx release`**: the ~140-line bash
  version job (affected detection, patch bumps, tagging) reimplements what
  `nx release` does natively — conventional-commit bumps, per-project
  independent versioning, changelogs, Cargo.toml + package.json support. Also
  closes the Python gap: `pyproject.toml` is not bumped today, so a uv/python
  monorepo cannot release. Do this BEFORE extracting the KCL package to
  kcl-packages so the shared package ships the good release flow.
- [ ] **`[skip ci]` policy**: the version-bump commit is never CI'd as-committed.
  Either accept explicitly, or move to a tag-triggered release flow.

## Suggested sequence

1. Resolve duplicate CI (running cost today).
2. Safety fixes: release concurrency group, `cargo update --workspace`,
   split build/scan failure handling.
3. Pin/auth hygiene: bun pin, installer pins, rust-toolchain.toml, WIF-only.
4. Structural: `release:` config section + `generate_monorepo_release` built on
   `nx release`; retire both handwritten workflows; then extract the package to
   kcl-packages (OCI) with the regen automation
   (repository_dispatch + schedule + drift check).
