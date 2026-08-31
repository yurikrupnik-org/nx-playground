/**
 * `scope:` tag for every graph node the plugin touches — the declared ownership
 * map that `just boundaries` (tools/nx/check-boundaries.ts) enforces.
 *
 * The scope is DECLARED here, never derived from the dependency graph: a tag
 * computed from who-depends-on-whom would follow every new edge, and a lint
 * keyed on it could never fail. Deriving from the path alone is also not
 * enough, because two facts cut across the directory layout:
 *
 * - `apps/zerg/tasks` is an extracted service with an authenticated gRPC
 *   boundary (see docs/adr-tasks-service-boundary.md). Its domain crate is
 *   private to it — `zerg_api` regaining a `domain_tasks` dependency is the
 *   regression backlog item 1.3 exists to block — so the service and its
 *   domain get their own scope, `tasks`, distinct from the `zerg` vertical
 *   that hosts them on disk. The wire contract (`libs/contracts/tasks`) stays
 *   `shared`: being consumable from outside the boundary is its entire job.
 * - `libs/domains/cloud_resources` is consumed by BOTH `zerg_api` and
 *   `terran_api` today, so it is honestly `shared` — this is the undecided
 *   bounded-context seam flagged in docs/architecture-review-todo.md Issue 5.
 *   When that call is made, retag it here and the gate enforces the decision.
 *
 * Everything else follows the layout: `apps/<vertical>/**` belongs to its
 * vertical, and every lib is `shared` — the modular-monolith rule that domains
 * are composed at the app layer. The rule the checker applies: an edge may
 * stay inside one scope or point at `shared`; `shared` may only depend on
 * `shared`. (`scripts/kcl/ci` hand-declares `scope:infra` in its project.json;
 * any scope name works — the checker only compares them.)
 */

/** Exact project roots whose scope departs from the path-derived default. */
const SCOPE_OVERRIDES: Record<string, string> = {
  'apps/zerg/tasks': 'tasks',
  'libs/domains/tasks': 'tasks',
  'libs/domains/todo': 'todo',
  'libs/domains/projects': 'zerg',
  'libs/domains/users': 'zerg',
  'libs/domains/vector': 'zerg',
  'libs/domains/cloud_resources': 'shared',
};

/** `scope:<name>` for a project root, e.g. `apps/todo/worker` -> `scope:todo`. */
export function scopeTag(dir: string): string {
  const override = SCOPE_OVERRIDES[dir];
  if (override) return `scope:${override}`;
  const [top, second] = dir.split('/');
  return top === 'apps' && second ? `scope:${second}` : 'scope:shared';
}

/**
 * Pre-existing edges the gate tolerates, each tied to an open decision — an
 * entry here is a documented debt, not a hole. Remove the entry the moment
 * the decision lands and the gate enforces it.
 *
 * `domain_cloud_resources -> domain_projects`: the SeaORM `belongs_to` FK
 * flagged in docs/architecture-review-todo.md Issue 5 ("the seam that would
 * hurt most"). `cloud_resources` is `shared` (terran + zerg both consume it)
 * while `projects` is zerg-owned, so this edge crosses contexts. The open
 * todo: decide one-context (merge scopes) vs two (replace the FK with an ID
 * reference + event).
 */
export const GRANDFATHERED: ReadonlySet<string> = new Set([
  'domain_cloud_resources -> domain_projects',
]);
