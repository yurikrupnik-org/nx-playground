/**
 * Shared reader for the two-level `butler.toml` config, plus the app predicates
 * every local nx plugin in this directory needs.
 *
 * Why a TS reader at all: the `container`/`scan` targets are inferred, so their
 * option values must be materialised while nx computes the project graph —
 * before any binary in this repo is guaranteed to be built. Shelling out to
 * `butler` there would put a cargo compile in front of every nx command.
 *
 * The values are therefore derived twice, once here and once in butler
 * (`apps/butler/cli/src/container.rs`), from the same `butler.toml` facts. That
 * is deliberate and guarded: `just container-check` runs
 * `butler container verify --graph <nx graph dump>`, which recomputes the facts
 * in Rust and diffs them against what this plugin put in the graph. Change one
 * side without the other and that gate fails.
 *
 * Zero dependencies on purpose (see the plugin headers): nx loads these files
 * directly, so the TOML subset below is hand-parsed rather than pulling a
 * parser into the graph-computation path.
 */

import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

export const CONFIG_FILE = 'butler.toml';

/**
 * nx's `createNodesV2` contract, spelled out so no plugin in this directory has
 * to depend on `@nx/devkit`. Each entry maps a matched file to the projects it
 * contributes targets (and tags) to; nx merges those onto the existing graph
 * nodes.
 */
export type ProjectContribution = {
  targets: Record<string, unknown>;
  tags?: string[];
};

export type CreateNodesV2 = [
  string,
  (
    files: readonly string[],
    options: unknown,
    context: { workspaceRoot: string },
  ) => Promise<[string, { projects: Record<string, ProjectContribution> }][]>,
];

/**
 * What an app is built from — the same rule as `tilt.rs::app_kind`:
 * `Cargo.toml` -> a compiled service, `vite.config.ts` -> a static SPA,
 * `astro.config.mjs` -> a Node SSR server.
 */
export type Kind = 'service' | 'web' | 'node';

/** The repo convention for a kind: `[imageDefaults.<kind>]`. */
export interface KindImage {
  dockerfile: string;
  /** Multi-stage build stage; falls back to the `[dockerfileTarget]` map. */
  target?: string;
  /** The single per-app build argument: `APP_NAME` / `DIST_PATH`. */
  buildArg?: string;
}

/** An app's `[image]`, for one that departs from the convention. */
export interface AppImage {
  dockerfile: string;
  context: string;
  target?: string;
  tag: string;
  buildArgs: Record<string, string>;
}

export interface RootConfig {
  /** Substituted for `$REGISTRY`; only its presence matters to the plugins. */
  registry: string;
  /** Deployment environment, i.e. which kustomize overlay marks a deployable app. */
  env: string;
  imageDefaults: Partial<Record<Kind, KindImage>>;
  /** `[dockerfileTarget]`: Dockerfile path -> stage, for an image pinning none. */
  dockerfileTarget: Record<string, string>;
  /**
   * `[k8s] outDir` with `{env}` expanded: where the rendered manifests land, and
   * therefore what the `k8s-gen` target must declare as its output. nx caches on
   * declared outputs, so this has to be the same string butler writes to.
   */
  k8sOutDir: string;
  /** `[container] extra`: apps that ship an image but no k8s manifests. */
  containerExtra: string[];
}

// ---------------------------------------------------------------------------
// TOML subset

/** Table header (dotted, `[[x]]` as `x[i]`) -> key -> raw value text. */
type Tables = Map<string, Map<string, string>>;

/** Drop a trailing `#` comment, respecting quoted strings. */
function stripComment(line: string): string {
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === '"') quoted = !quoted;
    else if (c === '#' && !quoted) return line.slice(0, i);
  }
  return line;
}

/** Unclosed `[` count outside strings, for arrays spanning several lines. */
function bracketDepth(text: string): number {
  let quoted = false;
  let depth = 0;
  for (const c of text) {
    if (c === '"') quoted = !quoted;
    else if (!quoted && c === '[') depth++;
    else if (!quoted && c === ']') depth--;
  }
  return depth;
}

function unquote(text: string): string {
  return text.startsWith('"') && text.endsWith('"') ? text.slice(1, -1) : text;
}

/**
 * The subset of TOML this repo's config files use: tables, dotted table names,
 * arrays of tables, quoted keys, string/bool/number scalars, and string arrays
 * (possibly spanning lines). Anything else is a loud error rather than a
 * silently ignored line — a plugin that guesses produces a wrong build.
 */
function parseToml(text: string, file: string): Tables {
  const tables: Tables = new Map();
  const arrayCounts = new Map<string, number>();
  const lines = text.split('\n');
  let current = '';

  const table = (name: string): Map<string, string> => {
    let t = tables.get(name);
    if (!t) {
      t = new Map();
      tables.set(name, t);
    }
    return t;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = stripComment(lines[i]).trim();
    if (!line) continue;

    if (line.startsWith('[[')) {
      const end = line.indexOf(']]');
      if (end < 0)
        throw new Error(`${file}:${i + 1}: unterminated table header`);
      const name = line.slice(2, end).trim();
      const index = arrayCounts.get(name) ?? 0;
      arrayCounts.set(name, index + 1);
      current = `${name}[${index}]`;
      // Register on the header, not on the first key: `[workload]` with no keys
      // of its own is legal TOML and is exactly how an app says "inherit the
      // whole kind default", so a lazily created table would read as absent.
      table(current);
      continue;
    }
    if (line.startsWith('[')) {
      const end = line.indexOf(']');
      if (end < 0)
        throw new Error(`${file}:${i + 1}: unterminated table header`);
      current = line.slice(1, end).trim();
      table(current);
      continue;
    }

    const match = /^("[^"]*"|[A-Za-z0-9_.-]+)\s*=\s*(.*)$/.exec(line);
    if (!match)
      throw new Error(`${file}:${i + 1}: unsupported TOML line \`${line}\``);
    let value = match[2].trim();
    while (bracketDepth(value) > 0) {
      i++;
      if (i >= lines.length) throw new Error(`${file}: unterminated array`);
      value += ` ${stripComment(lines[i]).trim()}`;
    }
    table(current).set(unquote(match[1]), value.trim());
  }
  return tables;
}

function requireString(
  tables: Tables,
  table: string,
  key: string,
  file: string,
): string {
  const raw = tables.get(table)?.get(key);
  if (raw === undefined) {
    const where = table ? `[${table}] ` : '';
    throw new Error(`${file}: missing ${where}\`${key}\``);
  }
  if (!raw.startsWith('"'))
    throw new Error(`${file}: \`${key}\` must be a string, got ${raw}`);
  return unquote(raw);
}

function stringArray(raw: string, file: string, key: string): string[] {
  const inner = raw.trim();
  if (!inner.startsWith('[') || !inner.endsWith(']')) {
    throw new Error(`${file}: \`${key}\` must be an array, got ${raw}`);
  }
  const out: string[] = [];
  let item = '';
  let quoted = false;
  for (const c of inner.slice(1, -1)) {
    if (c === '"') quoted = !quoted;
    if (c === ',' && !quoted) {
      if (item.trim()) out.push(unquote(item.trim()));
      item = '';
      continue;
    }
    item += c;
  }
  if (item.trim()) out.push(unquote(item.trim()));
  return out;
}

// ---------------------------------------------------------------------------
// Config

export function readRootConfig(workspaceRoot: string): RootConfig {
  const file = CONFIG_FILE;
  const tables = parseToml(
    readFileSync(join(workspaceRoot, file), 'utf8'),
    file,
  );

  const imageDefaults: Partial<Record<Kind, KindImage>> = {};
  for (const kind of ['service', 'web', 'node'] as const) {
    const name = `imageDefaults.${kind}`;
    if (!tables.has(name)) continue;
    const stage = tables.get(name)?.get('target');
    const buildArg = tables.get(name)?.get('buildArg');
    imageDefaults[kind] = {
      dockerfile: requireString(tables, name, 'dockerfile', file),
      target: stage === undefined ? undefined : unquote(stage),
      buildArg: buildArg === undefined ? undefined : unquote(buildArg),
    };
  }

  const dockerfileTarget: Record<string, string> = {};
  for (const [dockerfile, stage] of tables.get('dockerfileTarget') ?? []) {
    dockerfileTarget[dockerfile] = unquote(stage);
  }

  const outDir = tables.get('k8s')?.get('outDir');
  const env = requireString(tables, '', 'env', file);
  const extra = tables.get('container')?.get('extra');
  return {
    registry: requireString(tables, '', 'registry', file),
    env,
    imageDefaults,
    dockerfileTarget,
    k8sOutDir: (outDir === undefined
      ? 'manifests/k8s/apps'
      : unquote(outDir)
    ).replace('{env}', env),
    containerExtra:
      extra === undefined ? [] : stringArray(extra, file, '[container] extra'),
  };
}

/** An app's `[image]`, or undefined when it follows the repo convention. */
export function readAppImage(
  workspaceRoot: string,
  appDir: string,
): AppImage | undefined {
  const path = join(workspaceRoot, appDir, CONFIG_FILE);
  if (!existsSync(path)) return undefined;
  const file = `${appDir}/${CONFIG_FILE}`;
  const tables = parseToml(readFileSync(path, 'utf8'), file);
  if (!tables.has('image')) return undefined;

  const buildArgs: Record<string, string> = {};
  for (const [key, value] of tables.get('image.buildArgs') ?? []) {
    buildArgs[key] = unquote(value);
  }
  const stage = tables.get('image')?.get('target');
  return {
    dockerfile: requireString(tables, 'image', 'dockerfile', file),
    context: unquote(tables.get('image')?.get('context') ?? '"."'),
    target: stage === undefined ? undefined : unquote(stage),
    tag: requireString(tables, 'image', 'tag', file),
    buildArgs,
  };
}

// ---------------------------------------------------------------------------
// App predicates

/**
 * Kind from what the app is built with; undefined = not an app (a manifest-only
 * directory such as `apps/zerg/shared`). Mirrors `tilt.rs::app_kind`.
 */
export function appKind(
  workspaceRoot: string,
  appDir: string,
): Kind | undefined {
  if (existsSync(join(workspaceRoot, appDir, 'Cargo.toml'))) return 'service';
  if (existsSync(join(workspaceRoot, appDir, 'vite.config.ts'))) return 'web';
  if (existsSync(join(workspaceRoot, appDir, 'astro.config.mjs')))
    return 'node';
  return undefined;
}

/**
 * Does the app declare a workload — the "this gets deployed, so it needs an
 * image" signal. It replaced "ships k8s manifests on disk": the manifests are
 * now GENERATED from this table through the `app` KCL package, so the presence
 * of the source is the fact, not the presence of its output.
 */
export function hasWorkload(workspaceRoot: string, appDir: string): boolean {
  const path = join(workspaceRoot, appDir, CONFIG_FILE);
  if (!existsSync(path)) return false;
  return parseToml(readFileSync(path, 'utf8'), `${appDir}/${CONFIG_FILE}`).has(
    'workload',
  );
}

/** A directory nx already treats as a project (inferred targets merge onto it). */
export function isProject(workspaceRoot: string, dir: string): boolean {
  return (
    existsSync(join(workspaceRoot, dir, 'project.json')) ||
    existsSync(join(workspaceRoot, dir, 'package.json')) ||
    existsSync(join(workspaceRoot, dir, 'Cargo.toml'))
  );
}

/**
 * Image repository name for an app that declares none: its path below `apps/`
 * joined by `-` (`apps/todo/api` -> `todo-api`), which is the naming this
 * repo's Deployments already use. Mirrors `tilt.rs::derived_image_name`.
 */
export function derivedImageName(appDir: string): string {
  const trimmed = appDir.startsWith('apps/')
    ? appDir.slice('apps/'.length)
    : appDir;
  return trimmed.replace(/\//g, '-');
}

/** A cargo crate's package name and whether it produces a binary. */
export interface CargoCrate {
  name: string;
  hasBinary: boolean;
}

/** undefined for a virtual manifest (a workspace root with no `[package]`). */
export function readCargoCrate(
  workspaceRoot: string,
  dir: string,
): CargoCrate | undefined {
  const path = join(workspaceRoot, dir, 'Cargo.toml');
  if (!existsSync(path)) return undefined;
  const tables = parseToml(readFileSync(path, 'utf8'), `${dir}/Cargo.toml`);
  const name = tables.get('package')?.get('name');
  if (name === undefined) return undefined;
  const declaresBin = [...tables.keys()].some((t) => t.startsWith('bin['));
  return {
    name: unquote(name),
    hasBinary:
      declaresBin ||
      existsSync(join(workspaceRoot, dir, 'src', 'main.rs')) ||
      existsSync(join(workspaceRoot, dir, 'src', 'bin')),
  };
}
