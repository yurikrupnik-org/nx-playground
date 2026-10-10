/**
 * Drift gate for the platform tool registry — `task tooling-check`.
 *
 * Usage: bun tools/tooling/check-registry.ts [registry.toml]
 *
 * The registry (docs/tooling/registry.toml, contract in
 * .claude/skills/cncf-manager/SKILL.md) claims, per tool, WHAT INSTALLS IT,
 * WHAT USES IT and WHICH GATE catches it breaking. This file is what stops
 * those claims from rotting the way the README's Istio/Kiali rows did:
 *
 *   1. every path cited in `install` / `usage` still resolves (globs allowed);
 *   2. `status = "adopted"` needs at least one `usage` citation;
 *   3. `gate = "none"` on an adopted row is a defect UNLESS the row is listed
 *      in exactly one `[[gap]]`, which must carry a `closes_when` and an owner;
 *   4. `paid = true` needs an `approval` (the human-in-the-loop record) —
 *      an agent may never add one on its own;
 *   5. names are unique and the enums are closed.
 *
 * It deliberately does NOT talk to a cluster: a repo-only gate that always runs
 * beats a cluster gate that never does. Runtime evidence is class 3 of the
 * drift audit and stays manual.
 */

import { existsSync, readFileSync } from 'node:fs';
import { argv, exit } from 'node:process';

const CATEGORIES: Record<string, true> = {
  cluster: true,
  gitops: true,
  networking: true,
  config: true,
  database: true,
  secrets: true,
  messaging: true,
  observability: true,
  security: true,
  build: true,
  'dev-loop': true,
  testing: true,
  other: true,
};

const STATUSES: Record<string, true> = {
  adopted: true,
  trial: true,
  candidate: true,
  rejected: true,
  external: true,
  claimed: true,
};

const CNCF: Record<string, true> = {
  graduated: true,
  incubating: true,
  sandbox: true,
  'not-cncf': true,
  unknown: true,
};

interface Tool {
  name: string;
  category: string;
  status: string;
  cncf: string;
  install: string;
  usage?: string[];
  key?: string;
  gate: string;
  paid: boolean;
  owner: string;
  note?: string;
  decision?: string;
  review?: string;
  approval?: { by?: string; date?: string; cap?: string; review?: string };
}

interface Gap {
  name: string;
  applies_to: string[];
  why: string;
  closes_when: string;
  owner: string;
}

const file = argv[2] ?? 'docs/tooling/registry.toml';
if (!existsSync(file)) {
  console.error(`tooling-check: ${file} not found`);
  exit(2);
}

const parsed = Bun.TOML.parse(await Bun.file(file).text()) as {
  tool?: Tool[];
  gap?: Gap[];
};
const tools = parsed.tool ?? [];
const gaps = parsed.gap ?? [];

const errors: string[] = [];
const err = (tool: string, msg: string) => errors.push(`${tool}: ${msg}`);

/**
 * Pull repo-path-looking tokens out of a free-form evidence string.
 * Registry-hosted refs (`oci://…`, `ghcr.io/…`), shell expansions and prose in
 * parentheses are not repo paths and are skipped — the parenthetical is where
 * a row explains itself, so it must never be mistaken for a citation.
 */
function paths(evidence: string): string[] {
  return evidence
    .replace(/\([^)]*\)/g, ' ')
    .split(/[\s,;]+/)
    .filter(
      (t) =>
        t.includes('/') &&
        !t.includes('://') &&
        !t.includes('$') &&
        !/^[a-z0-9-]+\.(io|com|org|dev|net)\//.test(t),
    )
    .map((t) => t.replace(/[.,;:]+$/, ''));
}

/** Globs are scanned, plain paths stat'd — a `*` in an evidence path is legal. */
const resolves = (p: string) =>
  p.includes('*') || p.includes('{')
    ? [...new Bun.Glob(p).scanSync('.')].length > 0
    : existsSync(p);

/**
 * `gate = "none — a cache miss only slows CI"` must not launder an ungated row
 * past the `[[gap]]` requirement: anything starting with "none" is ungated.
 */
const ungated = (gate: string) => /^none\b/i.test(gate.trim());

const seen = new Set<string>();
for (const t of tools) {
  const id = t.name ?? '<unnamed>';
  for (const field of [
    'name',
    'category',
    'status',
    'cncf',
    'install',
    'gate',
    'owner',
  ] as const) {
    if (typeof t[field] !== 'string' || t[field] === '')
      err(id, `missing required field \`${field}\``);
  }
  if (typeof t.paid !== 'boolean') err(id, 'missing required field `paid`');
  if (seen.has(id)) err(id, 'duplicate name');
  seen.add(id);

  if (!CATEGORIES[t.category]) err(id, `unknown category \`${t.category}\``);
  if (!STATUSES[t.status]) err(id, `unknown status \`${t.status}\``);
  if (!CNCF[t.cncf]) err(id, `unknown cncf \`${t.cncf}\``);
  if (!t.owner?.startsWith('scope:')) err(id, 'owner must be a `scope:` tag');

  for (const p of [
    ...paths(t.install ?? ''),
    ...(t.usage ?? []).flatMap(paths),
  ])
    if (!resolves(p)) err(id, `evidence path does not exist: ${p}`);
  if (t.decision?.includes('/') && !resolves(t.decision))
    err(id, `decision doc does not exist: ${t.decision}`);
  // `key` replaces the line number the registry deliberately omits, so it has
  // to be checked or a version pin (`cnpg-1.24.0.yaml`) rots silently. Match
  // token-wise: a `key` may be prose that spans several lines of the file.
  const keyFile = paths(t.install ?? '').find(
    (p) => !p.includes('*') && existsSync(p),
  );
  if (keyFile && t.key) {
    const text = readFileSync(keyFile, 'utf8');
    const missing = (t.key.replace(/\([^)]*\)/g, ' ').match(/\S{4,}/g) ?? [])
      .map((tok) => tok.replace(/^[`"']+|[`"',.;]+$/g, ''))
      .filter((tok) => tok.length >= 4 && !text.includes(tok));
    if (missing.length > 0)
      err(id, `key token(s) not found in ${keyFile}: ${missing.join(', ')}`);
  }

  if (t.status === 'adopted' && (t.usage ?? []).length === 0)
    err(id, 'status=adopted with no `usage` evidence');
  if (t.paid && !t.approval?.by)
    err(id, 'paid=true requires an `approval` record (human decision)');
  if (!t.paid && t.approval) err(id, 'approval on a free tool');
  if (t.status === 'claimed' && !ungated(t.gate))
    err(id, 'status=claimed cannot have a gate');
}

const gapped = new Map<string, string>();
for (const g of gaps) {
  if (!g.name || !g.closes_when || !g.owner?.startsWith('scope:'))
    errors.push(
      `gap ${g.name ?? '<unnamed>'}: needs name, closes_when, scope: owner`,
    );
  for (const name of g.applies_to ?? []) {
    if (!seen.has(name)) errors.push(`gap ${g.name}: unknown tool \`${name}\``);
    const prev = gapped.get(name);
    if (prev)
      errors.push(`gap ${g.name}: \`${name}\` already declared by ${prev}`);
    gapped.set(name, g.name);
  }
}

for (const t of tools) {
  if (t.status !== 'adopted' || !ungated(t.gate)) continue;
  if (!gapped.has(t.name))
    err(
      t.name,
      'status=adopted with gate=none and no [[gap]] declaring it — add a gate or declare the gap',
    );
}
for (const [name, gap] of gapped) {
  const t = tools.find((x) => x.name === name);
  if (t && !(t.status === 'adopted' && ungated(t.gate)))
    errors.push(
      `gap ${gap}: \`${name}\` is gated or not adopted — drop it from applies_to`,
    );
}

if (errors.length > 0) {
  console.error(`tooling-check: ${errors.length} problem(s) in ${file}`);
  for (const e of errors) console.error(`  ${e}`);
  exit(1);
}

const by = (s: string) => tools.filter((t) => t.status === s).length;
console.log(
  `tooling-check: ${tools.length} tools ` +
    `(${by('adopted')} adopted, ${by('trial')} trial, ${by('external')} external, ` +
    `${by('candidate')} candidate, ${by('rejected')} rejected, ${by('claimed')} claimed), ` +
    `${tools.filter((t) => t.paid).length} paid with approvals, ` +
    `${gaps.length} declared gaps, every cited path resolves`,
);
