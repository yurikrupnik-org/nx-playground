# Python workspace recipes (uv + ruff + pytest) — imported by the root justfile.
#
# Scoped by TAG, never by project name: `apps/test-cli1/project.json` carries
# `["python", "lang:python"]`, so a second python project is gated the day it is
# generated, with no edit here. `tag:lang:python` is the selector — the bare
# `python` tag is the human-readable half of the same pair.
#
# Routed through nx, unlike the Rust leaves. The "never run a whole-workspace
# task through nx" rule in AGENTS.md is a MEASUREMENT about cargo (per-crate
# processes serialize on the target-dir lock and re-check the shared dep
# closure); none of it applies to ruff/pytest, which are per-project processes
# either way. What nx buys here is real:
#   * the `test` target sets `cwd: {projectRoot}`, and the pytest `addopts` in
#     apps/test-cli1/pyproject.toml are `../../`-relative — a root-level
#     `uv run pytest` writes coverage/ and reports/ OUTSIDE the repo;
#   * `lint` is scoped by `lintFilePatterns`, so ruff never wanders into
#     platform/sdk/python or manifests/dockers/**/*.py, which are not uv
#     workspace members and were never linted;
#   * Nx Cloud caches all three across runs.
# uv is still the native tool underneath: every @nxlv/python executor shells out
# to `uv run`. `uv sync --locked` (CI, below) is the lockfile gate.

# ruff check over every project tagged lang:python
[group('quality')]
lint-py:
    bun nx run-many -t lint -p tag:lang:python

# nx build+test for every project tagged lang:python — `build` is folded in for
# the same reason `test-web` runs it: it is the only gate on PACKAGING. Proven,
# not assumed — delete apps/test-cli1/README.md (declared as `[project].readme`)
# and `nx test` still exits 0 while `nx build` fails with
# "Call to `uv_build.build_sdist` failed". pytest never touches the wheel.
[group('quality')]
test-py:
    bun nx run-many -t build test -p tag:lang:python

# ruff format --write over every project tagged lang:python
[group('quality')]
fmt-py:
    bun nx run-many -t format -p tag:lang:python

# ruff format --check — the read-only half, for `just fmt-check`
[group('quality')]
fmt-check-py:
    bun nx run-many -t format -p tag:lang:python --check

# Preview outdated python deps (read-only); `just upkg` does the actual refresh
[group('deps')]
outdated-py:
    uv lock --upgrade --dry-run
