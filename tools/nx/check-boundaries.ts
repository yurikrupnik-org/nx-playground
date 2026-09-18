/**
 * Tag-based dependency boundary gate over the nx graph — `just boundaries`.
 *
 * Usage: bun tools/nx/check-boundaries.ts <graph.json>
 * where <graph.json> is the output of `bun nx graph --file=<graph.json>`.
 *
 * One rule, applied to every edge (cargo edges from @monodon/rust, TS edges
 * from package.json workspace deps — both ecosystems in one graph):
 *
 *   an edge may stay inside its scope, or point at `scope:shared`;
 *   `scope:shared` may only depend on `scope:shared`.
 *
 * Scopes are declared in tools/nx/scope-tags.ts (contributed by the plugin)
 * plus the hand-written `tags` of the remaining project.json files. A node
 * with no `scope:` tag defaults to `shared` — the STRICTEST scope as a
 * source (it may depend on nothing vertical-owned), so an untagged newcomer
 * cannot silently reach into a vertical.
 *
 * This is backlog item 1.3 (docs/architecture-backlog.md) made executable:
 * `zerg_api` regaining `domain_tasks` (`scope:tasks`) fails this gate.
 *
 * Known over-approximation, accepted: @monodon/rust flattens [dependencies]
 * and [dev-dependencies] into one edge type (see AGENTS.md), so a dev-only
 * dependency counts as a real edge here. That is the conservative direction —
 * it can only make the gate stricter, never let a violation through.
 */

import { readFileSync } from 'node:fs';
import { argv, exit } from 'node:process';

import { GRANDFATHERED } from './scope-tags.ts';

interface GraphNode {
  name: string;
  data: { root: string; tags?: string[] };
}

interface GraphFile {
  graph: {
    nodes: Record<string, GraphNode>;
    dependencies: Record<string, { source: string; target: string }[]>;
  };
}

const file = argv[2];
if (!file) {
  console.error('usage: bun tools/nx/check-boundaries.ts <graph.json>');
  exit(2);
}

const { graph }: GraphFile = JSON.parse(readFileSync(file, 'utf8'));

function scope(name: string): string {
  const tags = graph.nodes[name]?.data.tags ?? [];
  const tag = tags.find((t) => t.startsWith('scope:'));
  return tag ? tag.slice('scope:'.length) : 'shared';
}

const violations: string[] = [];
for (const edges of Object.values(graph.dependencies)) {
  for (const { source, target } of edges) {
    // External packages (`npm:*`) and anything else without a node are not
    // workspace projects; the boundary rule has nothing to say about them.
    if (!graph.nodes[source] || !graph.nodes[target]) continue;
    if (GRANDFATHERED.has(`${source} -> ${target}`)) continue;
    const from = scope(source);
    const to = scope(target);
    if (from === to || to === 'shared') continue;
    violations.push(
      `${source} (scope:${from}) -> ${target} (scope:${to}): ` +
        (from === 'shared'
          ? 'a shared lib may not depend on a vertical-owned project'
          : `scope:${from} may only depend on scope:${from} or scope:shared`),
    );
  }
}

if (violations.length > 0) {
  console.error(`boundary violations (${violations.length}):`);
  for (const v of violations) console.error(`  ${v}`);
  exit(1);
}
console.log(
  `boundaries: ${Object.keys(graph.nodes).length} projects, every edge within scope`,
);
