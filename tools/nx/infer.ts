/**
 * Runs this repo's nx plugin (`plugin.ts`) for butler instead of for nx — the
 * example of a butler EXTERNAL INFERRER (`butler --infer 'bun tools/nx/infer.ts'`,
 * or `butler.toml` `[graph] infer`). The default is butler's native Rust port
 * of the plugin (`apps/butler/cli/src/infer/native`), which needs no node
 * process; this adapter exists so a repo whose inference lives in a TypeScript
 * nx plugin can feed butler without porting it, and so `butler graph verify`
 * can tell an nx-core mismatch (drift here too) from a port bug (drift only
 * natively).
 *
 * Protocol (spec: `apps/butler/cli/src/infer/external.rs`): butler appends the
 * phase as the last argument, writes JSON to stdin, reads JSON from stdout.
 *
 *   nodes         {workspaceRoot, files: every workspace file}
 *                 -> the createNodesV2 result, over the files the plugin's own
 *                    glob matches (nx does that filtering, so this does)
 *   dependencies  {workspaceRoot, projects: {name: {root}}}
 *                 -> the createDependencies result, each edge the source
 *                    declares only under [dev-dependencies] marked `dev: true`
 *                    (butler-only: nx has no such field, image build contexts
 *                    skip those edges)
 *
 * Zero dependencies, like the plugin it wraps.
 */

import { type CargoCrate, readCargoCrate } from './butler-config.ts';
import { createDependencies, createNodesV2 } from './plugin.ts';

/**
 * `{a,{b,c}d}` -> `a`, `bd`, `cd`. Nested groups matter: the plugin braces one
 * glob per inference module into a single pattern, and some of those globs
 * brace their own alternatives.
 */
function expandBraces(glob: string): string[] {
  let depth = 0;
  let open = -1;
  const commas: number[] = [];
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === '{') {
      if (depth++ === 0) open = i;
    } else if (c === ',' && depth === 1) {
      commas.push(i);
    } else if (c === '}' && depth > 0 && --depth === 0) {
      const cuts = [open, ...commas, i];
      const head = glob.slice(0, open);
      const tail = glob.slice(i + 1);
      return cuts
        .slice(1)
        .flatMap((cut, k) =>
          expandBraces(head + glob.slice(cuts[k] + 1, cut) + tail),
        );
    }
  }
  if (depth !== 0 || glob.includes('}'))
    throw new Error(`unbalanced braces in glob ${glob}`);
  return [glob];
}

/**
 * One brace-free glob as nx's native matcher reads it: `*` and `?` stay inside
 * one path segment, `**` spans zero or more whole segments, and a leading dot
 * is an ordinary character. Syntax this does not implement throws, so a plugin
 * adopting it fails loudly instead of being handed the wrong files.
 */
function globRegExp(glob: string): RegExp {
  if (/[[\]()!\\]/.test(glob))
    throw new Error(`unsupported glob syntax in ${glob}`);
  const segments = glob.split('/');
  const source = segments
    .map((segment, i) => {
      const last = i === segments.length - 1;
      if (segment === '**') return last ? '.*' : '(?:[^/]*/)*';
      if (segment.includes('**'))
        throw new Error(`\`**\` must be a whole segment in ${glob}`);
      const body = segment
        .replace(/[.+^${}|]/g, '\\$&')
        .replaceAll('*', '[^/]*')
        .replaceAll('?', '[^/]');
      return last ? body : `${body}/`;
    })
    .join('');
  return new RegExp(`^${source}$`);
}

/**
 * nx hands a plugin its files ordered path component by component
 * (`web/butler.toml` before `web-astro/butler.toml`), not bytewise as git
 * lists them. The plugin's first-contribution-per-dir rules see that order.
 */
function byComponents(a: string, b: string): number {
  const x = a.split('/');
  const y = b.split('/');
  for (let i = 0; i < Math.min(x.length, y.length); i++) {
    if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1;
  }
  return x.length - y.length;
}

async function stdinJson<T>(): Promise<T> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) chunks.push(chunk as Buffer);
  return JSON.parse(Buffer.concat(chunks).toString('utf8')) as T;
}

const phase = process.argv.at(-1);
if (phase === 'nodes') {
  const { workspaceRoot, files } = await stdinJson<{
    workspaceRoot: string;
    files: string[];
  }>();
  const [pattern, createNodes] = createNodesV2;
  const matchers = expandBraces(pattern).map(globRegExp);
  const matched = files
    .filter((file) => matchers.some((m) => m.test(file)))
    .sort(byComponents);
  const result = await createNodes(matched, undefined, { workspaceRoot });
  process.stdout.write(JSON.stringify(result));
} else if (phase === 'dependencies') {
  const context = await stdinJson<Parameters<typeof createDependencies>[1]>();
  const crates = new Map<string, CargoCrate | undefined>();
  const crateOf = (project: string): CargoCrate | undefined => {
    const { root } = context.projects[project];
    if (!crates.has(root))
      crates.set(root, readCargoCrate(context.workspaceRoot, root));
    return crates.get(root);
  };
  const edges = createDependencies(undefined, context).map((edge) => {
    const target = crateOf(edge.target)?.name;
    const devOnly =
      target !== undefined && crateOf(edge.source)?.devOnly.has(target);
    return devOnly ? { ...edge, dev: true } : edge;
  });
  process.stdout.write(JSON.stringify(edges));
} else {
  console.error(
    `usage: infer.ts nodes|dependencies (JSON on stdin), got ${phase}`,
  );
  process.exitCode = 2;
}
