# Web/node workspace recipes (nx + biome + ncu) — imported by the root justfile.
# nx handles web: package.json scripts as cached targets, scoped with -p '*-web'.

# Biome (root config) over all JS/TS/JSON/CSS, read-only
[group('quality')]
lint-web:
    bunx biome ci .

# nx build+test for every *-web project (vite build is the TS type gate)
[group('quality')]
test-web:
    bun nx run-many -t build test -p '*-web'

# Web auto-fix: biome check --write via each app's lint script
[group('quality')]
web-fix:
    bun nx run-many -t lint -p '*-web'

# npm-check-updates preview across all workspace package.json files (no -u)
[group('deps')]
outdated-node:
    -ncu --workspaces --target latest

# Run zerg web dev server
[group('dev')]
web:
    cd apps/zerg/web && bun run dev
