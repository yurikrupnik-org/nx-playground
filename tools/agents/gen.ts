/**
 * Agent skill renderer — `task agents-gen` / `task agents-check` / `task agents-export`.
 *
 * One corpus, four runtimes. The procedure for every skill is hand-written in
 * `.claude/skills/<name>/SKILL.md`; `docs/agents/registry.toml` states why that
 * skill exists, which gate proves it, and which runtimes it is exported to.
 * Everything below is DERIVED from those two inputs:
 *
 *   committed (drift-gated by `--check`)
 *     .gemini/GEMINI.md                      skill index for the Gemini CLI
 *     .gemini/commands/skill/<name>.toml     /skill:<name>, @-imports the SKILL.md
 *     docs/agents/README.md                  index for GitHub
 *     docs/agents/skills.html                the human explainer (why we have these)
 *
 *   build output (gitignored, `--export`)
 *     dist/agents/<platform>/instructions/<name>.md   hosted-agent instruction
 *     dist/agents/<platform>/knowledge/<name>.md      knowledge-base document
 *     dist/agents/<platform>/README.md                the exact upload commands
 *
 * A hosted agent (Bedrock, Vertex) has no repo checkout, so it gets two files:
 * a SHORT instruction (Bedrock caps `instruction` at 4000 chars — CreateAgent
 * API reference) that routes and forbids, plus the full SKILL.md as a retrieval
 * document. The instruction length is validated here, so a `why` that outgrows
 * the platform limit fails the gate instead of the deploy.
 *
 * Zero deps: Bun's TOML parser, Bun.Glob, node:fs.
 */

import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
} from 'node:fs';
import { dirname, join } from 'node:path';
import { argv, exit } from 'node:process';

const REGISTRY = 'docs/agents/registry.toml';
const SKILL_DIR = '.claude/skills';
const EXPORT_DIR = 'dist/agents';

interface Skill {
  name: string;
  status: string;
  owner: string;
  targets: string[];
  gate: string;
  why: string;
  evidence: string[];
  review?: string;
}

/** A registry row joined with its SKILL.md — the only shape the renderers see. */
interface LiveSkill extends Skill {
  description: string;
  body: string;
}

interface Gap {
  name: string;
  applies_to: string[];
  why: string;
  closes_when: string;
  owner: string;
}

interface Platform {
  label: string;
  /** How the runtime receives the skill, one line, for the explainer table. */
  delivery: string;
  /** Committed adapter path pattern, or '' when the export is build output. */
  artifact: string;
  /** Hosted runtimes only: hard cap on the instruction string. */
  instructionMax?: number;
  instructionMin?: number;
}

/**
 * `vertex` carries Bedrock's 4000 on purpose: ADK publishes no hard cap on
 * `instruction`, so the binding constraint is the other platform's, and one
 * instruction text then serves both.
 */
const PLATFORMS: Record<string, Platform> = {
  'claude-code': {
    label: 'Claude Code',
    delivery:
      'Native skill, auto-loaded when the description matches the request',
    artifact: '.claude/skills/<name>/SKILL.md',
  },
  'gemini-cli': {
    label: 'Gemini CLI',
    delivery:
      '/skill:<name> custom command; @{…} injects the same SKILL.md verbatim',
    artifact: '.gemini/commands/skill/<name>.toml',
  },
  bedrock: {
    label: 'AWS Bedrock',
    delivery:
      'Hosted agent: instruction + the SKILL.md as a knowledge-base document',
    artifact: 'dist/agents/bedrock/ (build output)',
    instructionMax: 4000,
    instructionMin: 40,
  },
  vertex: {
    label: 'GCP Vertex AI (ADK)',
    delivery: 'Hosted ADK agent: instruction + the SKILL.md in a RAG corpus',
    artifact: 'dist/agents/vertex/ (build output)',
    instructionMax: 4000,
    instructionMin: 40,
  },
};

const STATUSES: Record<string, true> = {
  adopted: true,
  trial: true,
  retired: true,
};

// ---------------------------------------------------------------- inputs ---

const errors: string[] = [];
const err = (who: string, msg: string) => errors.push(`${who}: ${msg}`);

if (!existsSync(REGISTRY)) {
  console.error(`agents: registry not found: ${REGISTRY}`);
  exit(2);
}

const parsed = Bun.TOML.parse(readFileSync(REGISTRY, 'utf8')) as {
  skill?: Skill[];
  gap?: Gap[];
};
const skills = parsed.skill ?? [];
const gaps = parsed.gap ?? [];

/** Frontmatter is two keys on one line each; anything else is a defect here. */
function frontmatter(
  text: string,
  id: string,
): { name: string; description: string; body: string } {
  const m = /^---\n([\s\S]*?)\n---\n([\s\S]*)$/.exec(text);
  if (!m) {
    err(id, 'SKILL.md has no `---` frontmatter block');
    return { name: '', description: '', body: text };
  }
  const field = (key: string) => {
    const hit = new RegExp(`^${key}:[ \\t]*(.+)$`, 'm').exec(m[1]);
    if (!hit) err(id, `frontmatter is missing \`${key}\``);
    return hit?.[1].trim() ?? '';
  };
  return {
    name: field('name'),
    description: field('description'),
    body: m[2].trim(),
  };
}

const live: LiveSkill[] = [];
for (const s of skills) {
  const id = s.name ?? '<unnamed>';
  for (const f of ['name', 'status', 'owner', 'gate', 'why'] as const)
    if (typeof s[f] !== 'string' || s[f] === '')
      err(id, `missing required field \`${f}\``);
  if (!STATUSES[s.status]) err(id, `unknown status \`${s.status}\``);
  if (!s.owner?.startsWith('scope:')) err(id, 'owner must be a `scope:` tag');
  if (!Array.isArray(s.targets) || s.targets.length === 0)
    err(id, 'needs at least one target');
  for (const t of s.targets ?? [])
    if (!PLATFORMS[t]) err(id, `unknown target \`${t}\``);
  if (s.status === 'trial' && !s.review)
    err(id, 'status=trial needs a `review` date');

  for (const p of s.evidence ?? [])
    if (!existsSync(p)) err(id, `evidence path does not exist: ${p}`);
  if ((s.evidence ?? []).length === 0 && s.status !== 'retired')
    err(id, 'needs `evidence` — a claim the reader can check');

  const path = `${SKILL_DIR}/${s.name}/SKILL.md`;
  if (s.status === 'retired') {
    if (existsSync(path))
      err(id, 'status=retired but the SKILL.md still exists');
    continue;
  }
  if (!existsSync(path)) {
    err(id, `no skill body at ${path}`);
    continue;
  }
  if (!s.targets?.includes('claude-code'))
    err(
      id,
      'a skill with a SKILL.md must target `claude-code` — that file IS the adapter',
    );

  const fm = frontmatter(readFileSync(path, 'utf8'), id);
  if (fm.name && fm.name !== s.name)
    err(id, `frontmatter name \`${fm.name}\` != registry name`);
  if (fm.description.length < 40)
    err(id, 'frontmatter description is too short to route on');
  live.push({ ...s, description: fm.description, body: fm.body });
}

const named = new Set(skills.map((s) => s.name));
for (const dir of existsSync(SKILL_DIR)
  ? readdirSync(SKILL_DIR, { withFileTypes: true })
  : [])
  if (dir.isDirectory() && !named.has(dir.name))
    err(dir.name, `${SKILL_DIR}/${dir.name} has no row in ${REGISTRY}`);

const gapped = new Map<string, Gap>();
for (const g of gaps) {
  if (!g.name || !g.closes_when || !g.owner?.startsWith('scope:'))
    errors.push(
      `gap ${g.name ?? '<unnamed>'}: needs name, closes_when, scope: owner`,
    );
  for (const n of g.applies_to ?? []) {
    if (!named.has(n)) errors.push(`gap ${g.name}: unknown skill \`${n}\``);
    if (gapped.has(n)) errors.push(`gap ${g.name}: \`${n}\` already declared`);
    gapped.set(n, g);
  }
}
const ungated = (gate: string) => /^none\b/i.test(gate.trim());
for (const s of skills) {
  if (s.status === 'retired') continue;
  if (ungated(s.gate) && !gapped.has(s.name))
    err(
      s.name,
      'gate=none with no [[gap]] declaring it — name a gate or declare the gap',
    );
  if (!ungated(s.gate) && gapped.has(s.name))
    err(
      s.name,
      `gap ${gapped.get(s.name)?.name} applies to a gated skill — drop it`,
    );
}

// --------------------------------------------------------------- render ---

const GEN =
  'GENERATED by tools/agents/gen.ts from docs/agents/registry.toml — do not edit';
/** Registry prose is wrapped for review; a prompt wants one paragraph. */
const flow = (s: string) => s.trim().replace(/\s*\n\s*/g, ' ');

function instruction(s: LiveSkill): string {
  const gap = gapped.get(s.name);
  const verify = ungated(s.gate)
    ? `No repo gate covers this skill (declared gap "${gap?.name}"): ${flow(gap?.why ?? '')} Say so in your output instead of implying it was checked.`
    : `Run \`${s.gate}\`. Nothing you propose is done until that command passes; report its output.`;
  return [
    `You are the "${s.name}" agent for the nx-playground monorepo: a Rust workspace plus Solid/Vite web apps, project graph by nx, builds by cargo, every task behind \`task\` (go-task).`,
    '',
    `WHEN YOU APPLY. ${s.description}`,
    '',
    `WHY YOU EXIST. ${flow(s.why)}`,
    '',
    `AUTHORITATIVE PROCEDURE. The document "${s.name}.md" in your knowledge base is a verbatim copy of .claude/skills/${s.name}/SKILL.md. Retrieve it and follow it literally before proposing anything — it encodes decisions that cannot be re-derived from the code. "repo-rules.md" (the repo's AGENTS.md) applies on top of it and wins over your instincts.`,
    '',
    'RULES THAT OUTRANK YOUR JUDGEMENT.',
    '- Reuse the convention that is already there. A second convention beside an existing one is a defect, never a style choice.',
    `- Cite the file that proves each claim: ${s.evidence.join(', ')}.`,
    `- ${verify}`,
    '- Never adopt a paid plan, seat, quota or SaaS token, never read or emit secret material, and never push to main. Escalate to a human instead.',
    '- If the procedure and this instruction disagree, the procedure wins and you say so.',
  ].join('\n');
}

/**
 * The `- Applies when:` and `- Verification:` lines are a parsed contract:
 * apps/skill-agents/skill_agents/catalog.py routes and reviews by them.
 */
function knowledge(s: LiveSkill): string {
  return [
    `<!-- ${GEN}. Source of truth: .claude/skills/${s.name}/SKILL.md -->`,
    `# Skill: ${s.name}`,
    '',
    `- Repository: nx-playground`,
    `- Applies when: ${s.description}`,
    `- Why it exists: ${flow(s.why)}`,
    `- Verification: ${s.gate}`,
    `- Owner: ${s.owner} · status: ${s.status}`,
    '',
    '---',
    '',
    s.body,
    '',
  ].join('\n');
}

/** TOML string: literal when it can be, basic with escapes when it cannot. */
function toml(value: string): string {
  if (!value.includes("'") && !value.includes('\n')) return `'${value}'`;
  const escaped = value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  return value.includes('\n')
    ? `"""\n${escaped.replace(/"""/g, '\\"\\"\\"')}\n"""`
    : `"${escaped}"`;
}

/**
 * The file injection is `@{path}`, NOT the bare `@path` that works in an
 * interactive prompt: custom commands run through AtFileProcessor, whose
 * trigger is `@{` (gemini-cli `prompt-processors/types.js`). A bare `@path`
 * is passed through verbatim and the model silently gets no procedure.
 */
function geminiCommand(s: LiveSkill): string {
  const prompt = [
    `You are working in the nx-playground monorepo. The skill below is the repo's own procedure, not a suggestion — follow it literally, and if it conflicts with your instinct, it wins.`,
    '',
    `Why this skill exists: ${flow(s.why)}`,
    ungated(s.gate)
      ? `Verification: no repo gate covers this (declared gap "${gapped.get(s.name)?.name}") — say so rather than implying it was checked.`
      : `Verification: run \`${s.gate}\` and report its output. Nothing is done until it passes.`,
    '',
    `@{.claude/skills/${s.name}/SKILL.md}`,
    '',
    'Task: {{args}}',
  ].join('\n');
  return [
    `# ${GEN}`,
    `# Skill body: .claude/skills/${s.name}/SKILL.md (edit there, then run \`task agents-gen\`)`,
    `description = ${toml(s.description)}`,
    `prompt = ${toml(prompt)}`,
    '',
  ].join('\n');
}

function geminiContext(): string {
  const rows = live
    .filter((s) => s.targets.includes('gemini-cli'))
    .map(
      (s) =>
        `| \`/skill:${s.name}\` | ${s.description} | \`.claude/skills/${s.name}/SKILL.md\` |`,
    );
  return [
    `<!-- ${GEN} -->`,
    '',
    '# Gemini CLI context — nx-playground',
    '',
    'The repo-wide rules are in `AGENTS.md`, loaded alongside this file via',
    '`context.fileName` in `.gemini/settings.json`. This file adds only the skill',
    'index: procedures that are too long to keep in context and are read on demand.',
    '',
    '## Skills',
    '',
    'Before acting on a request that matches a row below, READ that skill file (or',
    'run its command) and follow it. These encode decisions that cannot be',
    're-derived from the code; guessing produces changes that pass review and break',
    'in the cluster.',
    '',
    '| Command | Use when | Procedure |',
    '| --- | --- | --- |',
    ...rows,
    '',
    'Why each of these exists, and which gate proves it: `docs/agents/skills.html`',
    '(source of truth `docs/agents/registry.toml`).',
    '',
  ].join('\n');
}

function readme(): string {
  // A code span survives GitHub's markdown sanitizer verbatim; bare prose does
  // not, so only the uncoded `delivery` text below gets its `<` escaped.
  const rows = live.map((s) => {
    const gate = ungated(s.gate)
      ? `none — gap \`${gapped.get(s.name)?.name}\``
      : `\`${s.gate}\``;
    const runtimes = s.targets.map((t) => PLATFORMS[t].label).join(', ');
    return `| [\`${s.name}\`](../../.claude/skills/${s.name}/SKILL.md) | ${s.status} | ${gate} | ${runtimes} |`;
  });
  return [
    `<!-- ${GEN} -->`,
    '',
    '# Agent skills',
    '',
    'One corpus, four runtimes. Every procedure is written once as',
    '`.claude/skills/<name>/SKILL.md`; `registry.toml` records why it exists, the',
    'gate that proves it and the runtimes it is exported to; `tools/agents/gen.ts`',
    'renders every adapter. **Nothing below is hand-edited** — `task agents-check`',
    'fails on drift, inside `task verify`.',
    '',
    '- **Why we have these skills (read this first): [`skills.html`](skills.html)**',
    '- Source of truth: [`registry.toml`](registry.toml) + the SKILL.md bodies',
    '- Regenerate: `task agents-gen` · verify: `task agents-check` · hosted export: `task agents-export`',
    '',
    '| Skill | Status | Gate | Runtimes |',
    '| --- | --- | --- | --- |',
    ...rows,
    '',
    '## Runtimes',
    '',
    '| Runtime | How it gets the skill | Artifact |',
    '| --- | --- | --- |',
    ...Object.entries(PLATFORMS).map(
      ([, p]) =>
        `| ${p.label} | ${p.delivery.replace(/</g, '&lt;')} | \`${p.artifact}\` |`,
    ),
    '',
    'Bedrock and Vertex have no checkout, so `task agents-export` writes a short',
    'instruction (routing + prohibitions, capped at the Bedrock 4000-character',
    'limit) plus the full SKILL.md as a retrieval document, into gitignored',
    '`dist/agents/`. Creating the hosted agents themselves costs money and is a',
    'human decision — see `skill://cncf-manager`.',
    '',
  ].join('\n');
}

const esc = (s: string) =>
  s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
/** Registry prose is markdown-ish: backticks and bold, nothing else. */
const rich = (s: string) =>
  esc(flow(s))
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');

function html(): string {
  const cards = live
    .map((s) => {
      const gap = gapped.get(s.name);
      return `      <article class="skill" id="${esc(s.name)}">
        <header>
          <h3><a href="../../.claude/skills/${esc(s.name)}/SKILL.md"><code>${esc(s.name)}</code></a></h3>
          <span class="badge ${esc(s.status)}">${esc(s.status)}</span>
          <span class="owner">${esc(s.owner)}</span>
        </header>
        <p class="why">${rich(s.why)}</p>
        <dl>
          <dt>Fires when</dt><dd>${rich(s.description)}</dd>
          <dt>Proven by</dt><dd>${
            gap
              ? `<span class="nogate">no gate</span> — declared gap <code>${esc(gap.name)}</code>: ${rich(gap.why)} <em>Closes when ${rich(gap.closes_when)}.</em>`
              : `<code>${esc(s.gate)}</code>`
          }</dd>
          <dt>Evidence</dt><dd>${s.evidence
            .map((p) => `<a href="../../${esc(p)}"><code>${esc(p)}</code></a>`)
            .join(' · ')}</dd>
          <dt>Runtimes</dt><dd>${Object.keys(PLATFORMS)
            .map(
              (p) =>
                `<span class="rt ${s.targets.includes(p) ? 'on' : 'off'}">${esc(PLATFORMS[p].label)}</span>`,
            )
            .join('')}</dd>
        </dl>
      </article>`;
    })
    .join('\n');

  const matrix = Object.entries(PLATFORMS)
    .map(
      ([, p]) =>
        `          <tr><td>${esc(p.label)}</td><td>${esc(p.delivery)}</td><td><code>${esc(p.artifact)}</code></td></tr>`,
    )
    .join('\n');

  return `<!doctype html>
<!-- ${GEN} -->
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Agent skills — why nx-playground has them</title>
<style>
  :root {
    color-scheme: light dark;
    --bg: #fbfbfa; --fg: #16181d; --muted: #5d6470; --line: #dcdfe4;
    --card: #fff; --accent: #8b3d00; --warn: #8a5a00;
  }
  @media (prefers-color-scheme: dark) {
    :root { --bg:#14161a; --fg:#e7e9ee; --muted:#9aa3b0; --line:#2b2f37; --card:#1b1e24; --accent:#ffb26b; --warn:#e7b25a; }
  }
  * { box-sizing: border-box; }
  body {
    margin: 0 auto; max-width: 62rem; padding: 3rem 1.25rem 6rem;
    background: var(--bg); color: var(--fg);
    font: 16px/1.65 ui-sans-serif, -apple-system, "Segoe UI", Roboto, sans-serif;
  }
  code, pre { font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace; font-size: .88em; }
  code { background: color-mix(in srgb, var(--fg) 8%, transparent); padding: .1em .35em; border-radius: 4px; }
  h1 { font-size: 2.1rem; line-height: 1.2; margin: 0 0 .4rem; letter-spacing: -.02em; }
  h2 { font-size: 1.3rem; margin: 3rem 0 .75rem; letter-spacing: -.01em; }
  h3 { font-size: 1.05rem; margin: 0; }
  .lede { font-size: 1.1rem; color: var(--muted); margin: 0 0 2rem; }
  a { color: inherit; text-decoration-color: var(--line); text-underline-offset: 3px; }
  a:hover { text-decoration-color: var(--accent); }
  table { border-collapse: collapse; width: 100%; margin: 1rem 0; }
  th, td { text-align: left; padding: .55rem .6rem; border-bottom: 1px solid var(--line); vertical-align: top; }
  th { font-size: .78rem; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); font-weight: 600; }
  .claim { border-left: 3px solid var(--accent); padding: .1rem 0 .1rem 1rem; margin: 1.25rem 0; color: var(--fg); }
  .claim p { margin: .4rem 0; }
  .skill { background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 1.1rem 1.25rem; margin: .85rem 0; }
  .skill header { display: flex; align-items: baseline; gap: .6rem; flex-wrap: wrap; }
  .skill h3 code { background: none; padding: 0; font-size: 1rem; }
  .badge { font-size: .7rem; text-transform: uppercase; letter-spacing: .07em; border: 1px solid var(--line); border-radius: 999px; padding: .1rem .5rem; color: var(--muted); }
  .badge.adopted { border-color: color-mix(in srgb, var(--accent) 50%, var(--line)); color: var(--accent); }
  .owner { font-size: .78rem; color: var(--muted); margin-left: auto; }
  .why { margin: .7rem 0 .9rem; }
  dl { display: grid; grid-template-columns: 8.5rem 1fr; gap: .3rem .9rem; margin: 0; font-size: .92rem; }
  dt { color: var(--muted); font-size: .78rem; text-transform: uppercase; letter-spacing: .05em; padding-top: .18rem; }
  dd { margin: 0; }
  .nogate { color: var(--warn); font-weight: 600; }
  .rt { display: inline-block; font-size: .75rem; border: 1px solid var(--line); border-radius: 999px; padding: .08rem .55rem; margin: 0 .3rem .3rem 0; }
  .rt.on { border-color: color-mix(in srgb, var(--accent) 45%, var(--line)); color: var(--accent); }
  .rt.off { color: var(--muted); opacity: .45; text-decoration: line-through; }
  footer { margin-top: 3.5rem; padding-top: 1.2rem; border-top: 1px solid var(--line); color: var(--muted); font-size: .85rem; }
</style>
</head>
<body>
<h1>Agent skills, and why this repo has them</h1>
<p class="lede">${live.length} procedures, one corpus, ${Object.keys(PLATFORMS).length} runtimes. Written once, exported everywhere, each one tied to a command that fails when it is ignored.</p>

<h2>The problem they solve</h2>
<p>
  Every expensive mistake in this repo has the same shape: the obvious action was
  wrong for a reason the code does not state. <code>cargo new</code> produces a crate that
  compiles and shadows the inferred nx targets. <code>cargo upgrade --incompatible</code>
  bulldozes a pin that exists precisely to be skipped. A migration applied without
  re-hashing <code>atlas.sum</code> is green locally and breaks the operator in the cluster.
  Writing KCL next to the app is the intuitive layout and yields
  <code>CannotFindModule</code>.
</p>
<div class="claim">
  <p>
    A model — any model — reproduces those mistakes, because each one is the
    <em>reasonable</em> move. The fix is not a bigger prompt: rules only work if
    they arrive at the moment of the decision and carry the failure that motivated
    them.
  </p>
</div>
<p>
  So the repo splits its knowledge in two. <a href="../../AGENTS.md"><code>AGENTS.md</code></a> holds
  what is true on every single turn and is always in context. A <strong>skill</strong> holds a
  procedure that is long, conditional and only occasionally relevant: it stays on
  disk, announces itself with a one-line trigger, and is read in full only when a
  request matches. Nine of them, below.
</p>

<h2>What makes a skill legitimate</h2>
<p>
  The same bar the platform-tool registry uses, for the same reason: a documented
  rule nobody can check rots into a claim. Every row in
  <a href="registry.toml"><code>docs/agents/registry.toml</code></a> must state the failure it
  prevents, cite files a reader can open, and name the command that goes red if the
  skill is ignored. A skill with no gate is not deleted — it is declared as a gap,
  with an owner and a closing condition, and it is shown as ungated here.
</p>

<h2>One corpus, four runtimes</h2>
<p>
  The procedure is authored once, as <code>.claude/skills/&lt;name&gt;/SKILL.md</code>. Every other
  runtime gets a generated adapter, so a fix to a procedure cannot reach one vendor
  and miss another.
</p>
<table>
  <thead><tr><th>Runtime</th><th>How it gets the skill</th><th>Artifact</th></tr></thead>
  <tbody>
${matrix}
  </tbody>
</table>
<p>
  Hosted agents have no checkout, so they get two pieces: a short instruction that
  routes and forbids (capped at Bedrock's 4000-character <code>instruction</code> limit, which
  the generator enforces), plus the full SKILL.md as a retrieval document. Producing
  those files is free and local — <code>task agents-export</code>; <em>creating</em> the hosted
  agents costs money and stays a human decision.
</p>

<h2>The skills</h2>
${cards}

<footer>
  Generated by <code>tools/agents/gen.ts</code> from <code>docs/agents/registry.toml</code> and the
  SKILL.md bodies. Regenerate with <code>task agents-gen</code>; <code>task agents-check</code> (inside
  <code>task verify</code>) fails if this file drifts from the registry. Do not edit it by hand.
</footer>
</body>
</html>
`;
}

// --------------------------------------------------------------- outputs ---

const committed = new Map<string, string>();
committed.set('.gemini/GEMINI.md', geminiContext());
committed.set('docs/agents/README.md', readme());
committed.set('docs/agents/skills.html', html());
for (const s of live)
  if (s.targets.includes('gemini-cli'))
    committed.set(`.gemini/commands/skill/${s.name}.toml`, geminiCommand(s));

for (const s of live)
  for (const p of s.targets) {
    const max = PLATFORMS[p].instructionMax;
    if (max === undefined) continue;
    const len = instruction(s).length;
    if (len > max)
      err(
        s.name,
        `${p} instruction is ${len} chars, limit ${max} — shorten \`why\``,
      );
    const min = PLATFORMS[p].instructionMin ?? 0;
    if (len < min)
      err(s.name, `${p} instruction is ${len} chars, minimum ${min}`);
  }

if (errors.length > 0) {
  console.error(`agents: ${errors.length} problem(s) in ${REGISTRY}`);
  for (const e of errors) console.error(`  - ${e}`);
  exit(1);
}

const mode = argv[2] ?? '--write';
const write = (path: string, body: string) => {
  mkdirSync(dirname(path), { recursive: true });
  Bun.write(path, body);
};

/** Adapters are a closed set: a retired skill must not leave its command behind. */
function orphans(): string[] {
  const dir = '.gemini/commands/skill';
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .map((f) => `${dir}/${f}`)
    .filter((p) => !committed.has(p));
}

if (mode === '--check') {
  const stale: string[] = [];
  for (const [path, body] of committed) {
    if (!existsSync(path)) stale.push(`missing: ${path}`);
    else if (readFileSync(path, 'utf8') !== body)
      stale.push(`stale:   ${path}`);
  }
  for (const o of orphans()) stale.push(`orphan:  ${o}`);
  if (stale.length > 0) {
    console.error(
      `agents-check: ${stale.length} generated file(s) out of sync`,
    );
    for (const s of stale) console.error(`  ${s}`);
    console.error('run `task agents-gen`');
    exit(1);
  }
  console.log(
    `agents-check: ${live.length} skills, ${committed.size} generated files in sync, ` +
      `${gaps.length} declared gap(s), every evidence path resolves`,
  );
  exit(0);
}

if (mode === '--export') {
  const hosted = Object.keys(PLATFORMS).filter(
    (p) => PLATFORMS[p].instructionMax !== undefined,
  );
  rmSync(EXPORT_DIR, { recursive: true, force: true });
  let files = 0;
  for (const p of hosted) {
    const mine = live.filter((s) => s.targets.includes(p));
    for (const s of mine) {
      write(
        join(EXPORT_DIR, p, 'instructions', `${s.name}.md`),
        `${instruction(s)}\n`,
      );
      write(join(EXPORT_DIR, p, 'knowledge', `${s.name}.md`), knowledge(s));
      files += 2;
    }
    write(
      join(EXPORT_DIR, p, 'knowledge', 'repo-rules.md'),
      `<!-- ${GEN}. Verbatim copy of AGENTS.md; edit there. -->\n# nx-playground repo rules\n\n${readFileSync('AGENTS.md', 'utf8')}`,
    );
    write(join(EXPORT_DIR, p, 'README.md'), uploadGuide(p, mine));
    files += 2;
  }
  console.log(
    `agents-export: ${files} files in ${EXPORT_DIR}/{${hosted.join(',')}} ` +
      `(instruction + knowledge per skill, plus repo-rules.md)`,
  );
  exit(0);
}

if (mode !== '--write') {
  console.error(
    `agents: unknown mode \`${mode}\` (--write | --check | --export)`,
  );
  exit(2);
}

for (const o of orphans()) rmSync(o);
for (const [path, body] of committed) write(path, body);
console.log(
  `agents-gen: ${committed.size} files from ${live.length} skills ` +
    `(${live.filter((s) => s.targets.includes('gemini-cli')).length} gemini commands)`,
);

// --------------------------------------------------------- upload guides ---

function uploadGuide(platform: string, mine: LiveSkill[]): string {
  const names = mine.map((s) => s.name);
  const common = [
    `<!-- ${GEN} -->`,
    '',
    `# ${PLATFORMS[platform].label} export`,
    '',
    `${names.length} skills: ${names.join(', ')}.`,
    '',
    '- `instructions/<name>.md` — the agent instruction: routing, prohibitions,',
    '  the verification command. Short on purpose.',
    '- `knowledge/<name>.md` — the full procedure, for retrieval.',
    '- `knowledge/repo-rules.md` — AGENTS.md, applies to every agent here.',
    '',
    'Regenerate with `task agents-export`. This directory is gitignored build',
    'output; never edit it — edit `.claude/skills/<name>/SKILL.md` or',
    '`docs/agents/registry.toml`.',
    '',
    '> Creating hosted agents is a paid, human-approved decision',
    '> (`skill://cncf-manager`). The commands below are the intended shape, to be',
    '> run by a human with credentials, not by an agent.',
    '',
  ];
  const steps =
    platform === 'bedrock'
      ? [
          '## Upload',
          '',
          '```bash',
          '# 1. knowledge corpus -> S3 (one prefix per agent, or one shared prefix)',
          'aws s3 sync dist/agents/bedrock/knowledge/ "s3://$BUCKET/nx-playground/skills/"',
          '',
          '# 2. one agent per skill; `instruction` is capped at 4000 chars (enforced by the generator)',
          'aws bedrock-agent create-agent \\',
          '  --agent-name nx-playground-<name> \\',
          '  --foundation-model "$MODEL_ID" \\',
          '  --agent-resource-role-arn "$ROLE_ARN" \\',
          '  --instruction "file://dist/agents/bedrock/instructions/<name>.md"',
          '',
          '# 3. attach the knowledge base that indexes the S3 prefix from step 1',
          'aws bedrock-agent associate-agent-knowledge-base \\',
          '  --agent-id "$AGENT_ID" --agent-version DRAFT \\',
          '  --knowledge-base-id "$KB_ID" \\',
          '  --description "nx-playground skill procedures"',
          '```',
          '',
          'Re-running `create-agent` after a skill edit is wrong — use',
          '`update-agent` with the regenerated instruction file, then',
          '`prepare-agent`.',
        ]
      : [
          '## Upload',
          '',
          '```bash',
          '# 1. knowledge corpus -> GCS, indexed by a Vertex AI RAG corpus / Search datastore',
          'gcloud storage rsync dist/agents/vertex/knowledge/ "gs://$BUCKET/nx-playground/skills/" --recursive',
          '```',
          '',
          '```python',
          '# 2. one ADK agent per skill; instruction is the generated file, verbatim.',
          '#    static_instruction, NOT instruction: ADK substitutes `{name}` from',
          '#    session state in `instruction`, and a skill text may contain braces.',
          'from pathlib import Path',
          'from google.adk.agents import Agent',
          '',
          'agent = Agent(',
          '    model="gemini-2.5-pro",',
          '    name="nx_playground_<name>",',
          '    static_instruction=Path("dist/agents/vertex/instructions/<name>.md").read_text(),',
          '    tools=[retrieval_tool],  # pointed at the corpus from step 1',
          ')',
          '```',
          '',
          'Deploy with Agent Engine; the instruction is regenerated by',
          '`task agents-export`, so redeploy rather than editing it in the console.',
          '',
          'To run these agents locally — standalone, or under the multi-stage',
          'supervisor on ADK or LangGraph — use `apps/skill-agents`',
          '(`uv run skill-agents run "…"`), which reads this directory.',
        ];
  return [...common, ...steps, ''].join('\n');
}
