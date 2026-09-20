# Web/node workspace recipes (nx + biome + ncu) — imported by the root justfile.
# nx handles web: package.json scripts as cached targets, scoped with -p '*-web'.

# Biome (root config) over all JS/TS/JSON/CSS, read-only
[group('quality')]
lint-web:
    bunx biome ci .

# nx build+test+typecheck for every *-web project plus the shared web libs.
# `typecheck` (tsc --noEmit) is the TS gate — `vite build` is NOT: esbuild strips
# types without checking them.
[group('quality')]
test-web:
    bun nx run-many -t build test typecheck -p '*-web' web-auth

# Web auto-fix: biome check --write via each app's lint script
[group('quality')]
web-fix:
    bun nx run-many -t lint -p '*-web'

# Native N-API addons (libs/native/*): cargo build via napi-cli + their node tests.
# nx-driven (unlike plain Rust crates) because the deliverable is a JS package:
# the .node artifact plus generated index.js/index.d.ts are nx build outputs.
[group('quality')]
test-napi:
    bun nx run-many -t build test -p '@native/*'

# Browser e2e for the todo vertical (apps/todo/e2e): Playwright boots a
# throwaway Postgres (docker), todo-api, and all three frontends on dedicated
# ports, then drives every delivery option against the one backend. Needs
# docker + cargo; the first run compiles two binaries and downloads Chromium
# (`playwright install` is idempotent). ~10s warm, 9 tests.
[group('quality')]
e2e:
    cd apps/todo/e2e && bunx playwright install chromium
    bun nx e2e todo-e2e

# Dependency boundaries over the whole nx graph (cargo + TS edges): every edge
# must stay inside its `scope:` tag or point at scope:shared. Tags are declared
# in tools/nx/scope-tags.ts; this is backlog 1.3 as an executable gate.
[group('quality')]
boundaries:
    bun nx graph --file=dist/boundaries-graph.json >/dev/null
    bun tools/nx/check-boundaries.ts dist/boundaries-graph.json

# Platform tool registry drift gate (bun, no nx): every tool in
# docs/tooling/registry.toml must still cite paths that exist, every `adopted`
# row must name a gate or a declared [[gap]], and every paid tool must carry a
# human approval. Contract: .claude/skills/cncf-manager/SKILL.md.
[group('quality')]
tooling-check:
    bun tools/tooling/check-registry.ts

# Agent skill adapters: render every runtime's copy from the SKILL.md bodies +
# docs/agents/registry.toml (.gemini commands + context, docs/agents/README.md,
# docs/agents/skills.html). Contract: tools/agents/gen.ts.
[group('quality')]
agents-gen:
    bun tools/agents/gen.ts --write

# Agent skill drift gate: every registry row still cites paths that exist, has
# a gate or a declared [[gap]], and every generated adapter matches what the
# registry would render right now. Run `just agents-gen` when it fails.
[group('quality')]
agents-check:
    bun tools/agents/gen.ts --check

# Hosted-agent export (gitignored dist/agents/): per-skill instruction (capped
# at Bedrock's 4000-char limit) + knowledge document for AWS Bedrock and GCP
# Vertex AI, plus the upload commands. Creating the agents costs money and is a
# human decision — see .claude/skills/cncf-manager/SKILL.md.
[group('quality')]
agents-export:
    bun tools/agents/gen.ts --export

# Freeze the STAGED diff into one review input both pre-commit passes read
# (file table + class per file + the gates that diff implies + the diff
# itself). Exits 1 on a hard stop (secret-shaped path). Protocol:
# .claude/skills/precommit-review/SKILL.md
[group('quality')]
review-bundle out="dist/review/bundle.md":
    bun tools/review/bundle.ts --out {{ out }}

# npm-check-updates preview across all workspace package.json files (no -u)
[group('deps')]
outdated-node:
    -ncu --workspaces --target latest

# Run zerg web dev server
[group('dev')]
web:
    cd apps/zerg/web && bun run dev
