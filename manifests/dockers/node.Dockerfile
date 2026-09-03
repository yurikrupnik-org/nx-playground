# syntax=docker/dockerfile:1
# Node (Astro SSR) application image. Same contract as the sibling image
# recipes in this directory, so k8s manifests and probes can treat every app
# alike:
#   - serves $APP_DIR on :8080 as an unprivileged user
#   - GET /health -> 200 (probes)
#   - SSR, not static: every request is rendered by the Node standalone server
#     that `astro build` emits (dist/server/entry.mjs + dist/client)
#   - upstream API address comes from the environment (TODO_API_URL), not baked
#
# The build context is the workspace root (`context: "."`) and the app is
# selected with APP_DIR (e.g. apps/todo/web-astro), the way rust.Dockerfile is
# parameterised by APP_NAME.
#
# Why a Rust toolchain lives in the builder: the app imports
# `@native/field-selector`, an N-API addon over libs/native/field-selector kept
# as a runtime `require` via `vite.ssr.external`. Rollup cannot inline a `.node`
# binary, and the developer's copy is macOS/arm64, so the addon MUST be
# compiled for the image's own platform here rather than copied from context
# (`**/*.node` is excluded from the context for exactly that reason).
ARG APP_DIR
ARG NODE_VERSION=26
ARG BUN_VERSION=1.4.0

# Debian trixie on both stages on purpose: the addon is a glibc cdylib, so the
# builder's libc must match the runtime's.
FROM rust:1-slim-trixie AS builder
ARG APP_DIR
ARG BUN_VERSION
RUN test -n "$APP_DIR" || (echo "APP_DIR not set" && false)

# curl+unzip are for the bun installer; rust:slim already carries the cc and
# libc-dev that napi-build needs.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends ca-certificates curl unzip
ENV BUN_INSTALL=/usr/local
RUN curl -fsSL https://bun.sh/install | bash -s "bun-v${BUN_VERSION}"

WORKDIR /app

# The bun workspace globs (apps/**/*, libs/*/*) and the cargo workspace both
# need the whole tree, so there is no manifest-only pre-install layer to peel
# off here.
COPY package.json bun.lock ./
COPY Cargo.toml Cargo.lock ./
COPY apps/ apps/
COPY libs/ libs/

RUN --mount=type=cache,target=/root/.bun/install/cache,sharing=locked \
    bun install --frozen-lockfile

# napi-cli emits libs/native/field-selector/field-selector.linux-<arch>-gnu.node
# next to the index.js loader that picks it at require time. `CARGO=cargo` is
# prefixed inside the package script on purpose (a CARGO env var from secret
# fetching would otherwise shadow the binary).
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/app/target,id=rust-target,sharing=locked \
    cd libs/native/field-selector && bun run build

RUN cd "$APP_DIR" && bun run build

# Collect the runtime dependency closure. `astro build` inlines everything it
# can, so what survives as a real import is only what the app marked
# `vite.ssr.external` (plus whatever those packages need). Copying the install
# tree instead would drag in 700+ MB of build-only toolchains, and bun's
# isolated layout makes a partial copy of it meaningless (every entry is a
# symlink into node_modules/.bun), so the closure is materialised here.
COPY <<'JS' /runtime-deps.mjs
import fs from 'node:fs';
import { builtinModules } from 'node:module';
import path from 'node:path';

const appDir = path.resolve(process.argv[2]);
const destNodeModules = path.resolve(process.argv[3]);
const builtins = new Set(builtinModules);
const NAME = /^(?:@[a-z0-9][^/]*\/)?[a-z0-9][^/]*$/i;

/** node_modules dirs node would search from `dir`, nearest first. */
function searchDirs(dir) {
  const dirs = [];
  for (let d = dir; ; d = path.dirname(d)) {
    dirs.push(path.join(d, 'node_modules'));
    if (d === path.dirname(d)) return dirs;
  }
}

/** Bare package names still imported/required by the built output. */
function externals(dir, found = new Set()) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      externals(p, found);
      continue;
    }
    if (!/\.(m?js|cjs)$/.test(entry.name)) continue;
    const src = fs.readFileSync(p, 'utf8');
    for (const [, spec] of src.matchAll(
      /(?:\bfrom\s*|\bimport\s*|\brequire\s*\(\s*)['"]([^'"\n]+)['"]/g,
    )) {
      if (spec.startsWith('.') || spec.startsWith('/') || spec.startsWith('node:')) continue;
      const parts = spec.split('/');
      const name = spec.startsWith('@') ? parts.slice(0, 2).join('/') : parts[0];
      // Bundled code also holds specifier-shaped template strings; skip those.
      if (builtins.has(name) || !NAME.test(name)) continue;
      found.add(name);
    }
  }
  return found;
}

const placed = new Map();

/** Copy `name` plus its dependency closure into `into`, nesting as resolved. */
function copy(name, from, into, chain) {
  const dir = from.map((nm) => path.join(nm, name)).find((p) => fs.existsSync(p));
  if (!dir) return false;
  const real = fs.realpathSync(dir);
  // Already on this branch, or hoisted at the same version: node finds it by
  // walking up from the nested position, so a second copy is dead weight.
  if (chain.has(real) || placed.get(name) === real) return true;
  const target = path.join(into, name);
  if (!fs.existsSync(target)) {
    fs.mkdirSync(path.dirname(target), { recursive: true });
    // Dereference: workspace packages and store entries are symlinks, and a
    // package's own node_modules is re-created from the walk below instead.
    fs.cpSync(real, target, {
      recursive: true,
      dereference: true,
      filter: (src) => path.basename(src) !== 'node_modules',
    });
  }
  if (!placed.has(name)) placed.set(name, real);
  const pkg = JSON.parse(fs.readFileSync(path.join(real, 'package.json'), 'utf8'));
  const deps = { ...pkg.dependencies, ...pkg.optionalDependencies };
  const next = new Set(chain).add(real);
  for (const dep of Object.keys(deps)) {
    copy(dep, searchDirs(real), path.join(target, 'node_modules'), next);
  }
  return true;
}

const wanted = [...externals(path.join(appDir, 'dist'))].sort();
const missing = wanted.filter((name) => !copy(name, searchDirs(appDir), destNodeModules, new Set()));
console.log(`runtime deps: ${wanted.join(', ') || '(none)'}`);
if (missing.length) {
  console.error(`not resolvable from ${appDir}: ${missing.join(', ')}`);
  process.exit(1);
}
JS
RUN bun /runtime-deps.mjs "$APP_DIR" /runtime/node_modules

# ============================================================================
# runtime — stage name is pinned by butler.toml ([imageDefaults.node].target).
# ============================================================================
FROM node:${NODE_VERSION}-trixie-slim AS runtime
ARG APP_DIR

WORKDIR /app/${APP_DIR}
COPY --from=builder /app/${APP_DIR}/dist ./dist
COPY --from=builder /app/${APP_DIR}/package.json ./package.json
COPY --from=builder /runtime/node_modules ./node_modules

# node:*-slim ships an unprivileged `node` user (UID 1000), so the image
# satisfies Kubernetes runAsNonRoot / restricted PSS without a useradd layer.
USER 1000:1000

ENV NODE_ENV=production \
    HOST=0.0.0.0 \
    PORT=8080
EXPOSE 8080

LABEL \
    org.opencontainers.image.title="${APP_DIR}" \
    org.opencontainers.image.source="playground" \
    org.opencontainers.image.description="Astro SSR (Node standalone) app from Nx monorepo" \
    security.non-root="true" \
    security.base-image="node:${NODE_VERSION}-trixie-slim"

CMD ["node", "dist/server/entry.mjs"]
