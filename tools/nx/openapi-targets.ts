/**
 * `openapi-gate` for a crate that WRITES a committed OpenAPI document.
 *
 * Four crates annotate their handlers with `#[utoipa::path]` and export the
 * assembled document from a test (`fn export_openapi_*`), so `docs/openapi/*`
 * is generated output, never hand-edited — the same convention as the ts-rs
 * `export_bindings_*` tests. The export runs inside the normal test suite,
 * which means a diff that changes an annotation REWRITES the document and the
 * suite still passes: the drift only becomes visible to whoever diffs the
 * working tree afterwards. `task openapi-check` does that for the whole
 * workspace, and it is in `task verify` — but `verify` is the local gate; the
 * CI cargo job runs the affected crates' targets, so without a per-crate gate
 * a stale document merges.
 *
 * It matters more than a usual generated-file gate: `x` (apps/x/cli) embeds
 * these documents at compile time and derives its whole command tree from them,
 * so a stale document is a CLI addressing routes the server no longer serves.
 *
 * Derived, not configured. A hand-written `openapi-gate` already existed in
 * `apps/zerg/api/project.json` and covered exactly one of the four crates —
 * the other three (`terran_api`, `todo_api`, `domain_todo`) drifted silently
 * for as long as they have existed. The rule below reads the same two facts the
 * code already states: the crate depends on `utoipa`, and some file under its
 * `src/` both defines an export test and names a `docs/openapi/*.json` path.
 *
 * `plugin.ts` owns the nx registration; this file is imported into that single
 * plugin worker rather than registered as a plugin of its own.
 */

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import type { CargoCrate } from './butler-config.ts';

/** The repo's naming convention for the test that writes a document. */
const EXPORT_FN = /\bfn export_openapi\w*\s*\(/;

/** A committed document path as it is spelled in the export call. */
const DOCUMENT = /docs\/openapi\/[A-Za-z0-9._-]+\.json/g;

/**
 * Documents the crate's own sources write, sorted and de-duplicated.
 *
 * Only files that define an export test are searched, so a crate that merely
 * READS the documents (`apps/x/cli`, which `include_str!`s all three) or names
 * one in a comment contributes nothing.
 */
function exportedDocuments(workspaceRoot: string, dir: string): string[] {
  const src = join(workspaceRoot, dir, 'src');
  if (!existsSync(src)) return [];
  const found = new Set<string>();
  // String entries, not Dirents: the relative path is all this needs, and
  // `Dirent.parentPath` is newer than the Node floor nx itself builds on.
  for (const relative of readdirSync(src, { recursive: true })) {
    if (typeof relative !== 'string' || !relative.endsWith('.rs')) continue;
    const text = readFileSync(join(src, relative), 'utf8');
    if (!EXPORT_FN.test(text)) continue;
    for (const match of text.matchAll(DOCUMENT)) found.add(match[0]);
  }
  return [...found].sort();
}

/**
 * `openapi-gate`, or undefined for the crates — the overwhelming majority —
 * that export no document.
 */
export function openapiTargets(
  workspaceRoot: string,
  dir: string,
  crate: CargoCrate | undefined,
): Record<string, unknown> | undefined {
  // No utoipa, no document: skips the source scan for 40 of the 45 crates.
  if (!crate?.dependencies.has('utoipa')) return undefined;
  const documents = exportedDocuments(workspaceRoot, dir);
  if (documents.length === 0) return undefined;

  return {
    'openapi-gate': {
      executor: 'nx:run-commands',
      cache: true,
      // The committed documents are inputs, not outputs: this target asserts
      // they are already what the annotations produce. That makes a hand-edit
      // of a document a cache miss, which is the point — `cargo test` would
      // overwrite it and `git diff` would then fail.
      inputs: [
        'default',
        '^default',
        'rustGlobals',
        ...documents.map((doc) => `{workspaceRoot}/${doc}`),
      ],
      outputs: [],
      options: {
        commands: [
          `cargo test --package ${crate.name} export_openapi`,
          `git diff --exit-code -- ${documents.join(' ')}`,
        ],
        // The diff reads what the test just wrote.
        parallel: false,
        cwd: '{workspaceRoot}',
      },
      metadata: {
        description: `Fail if ${documents.join(', ')} has drifted from ${crate.name}'s annotations`,
        technologies: ['rust'],
      },
    },
  };
}
