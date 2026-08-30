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

# npm-check-updates preview across all workspace package.json files (no -u)
[group('deps')]
outdated-node:
    -ncu --workspaces --target latest

# Run zerg web dev server
[group('dev')]
web:
    cd apps/zerg/web && bun run dev
