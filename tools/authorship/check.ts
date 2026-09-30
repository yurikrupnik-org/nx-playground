#!/usr/bin/env bun
/**
 * Attribution gate — `Assisted-by:` trailers against the agent registry
 * (libs/core/authorship/agents.json).
 *
 *   bun tools/authorship/check.ts commit-msg <msg-file>
 *     lefthook `commit-msg`. Fails when an agent shell commits without
 *     crediting itself (the `prepare-commit-msg` stamp normally adds it), or
 *     when any `Assisted-by` value is not `<known-agent-id>[:<model>]`.
 *     Merges and replays (rebase, cherry-pick) only get the value check.
 *
 *   bun tools/authorship/check.ts range <rev-range> [--summary]
 *     CI / `task authorship-check RANGE=a..b` / `task authorship-report`.
 *     Validates every `Assisted-by` value of every non-merge commit in the
 *     range and prints one row per commit (sha, date, author and kind by the
 *     identity rules of core_authorship, attribution, assists), or with
 *     `--summary` one row per author.
 *
 * CI only sees what the commit says: an agent commit made with the hooks
 * missing or bypassed carries no trailer and looks exactly like a human one.
 * Only the local hooks can detect the agent shell; the range check proves the
 * trailers that ARE there are well-formed and name registered agents.
 */

import { readFileSync } from 'node:fs';
import { argv, exit } from 'node:process';
import { cleanMessage, git, replaying } from './git';
import {
  type Classification,
  classifyCommit,
  credits,
  detectAgent,
  isKnownAgent,
  parseTrailers,
  REGISTRY,
  splitAssist,
} from './registry';

const KNOWN = [REGISTRY.genericAgentId, ...REGISTRY.agents.map((a) => a.id)];
const AGENT_ID = /^[a-z0-9][a-z0-9-]*$/;

/** Problems with the `Assisted-by` trailers of `message`; empty when valid. */
export function trailerErrors(message: string): string[] {
  const errors: string[] = [];
  for (const t of parseTrailers(message)) {
    if (t.key.toLowerCase() !== REGISTRY.trailer.toLowerCase()) continue;
    const { agent, model } = splitAssist(t.value);
    const line = `'${t.key}: ${t.value}'`;
    if (!AGENT_ID.test(agent)) {
      errors.push(
        `${line}: the agent must be a lowercase registry id, not '${agent}'`,
      );
    } else if (!isKnownAgent(agent)) {
      errors.push(`${line}: unknown agent '${agent}'`);
    }
    if (t.value.includes(':') && (model === null || /\s/.test(model))) {
      errors.push(`${line}: the model after ':' must be one non-empty word`);
    }
  }
  return errors;
}

/**
 * Problems with a commit about to be made (commit-msg hook). `agentId` is the
 * agent detected in the committing shell; `replay` says why the commit is
 * exempt from crediting it (merge, rebase, cherry-pick).
 */
export function commitMsgErrors(
  message: string,
  agentId: string | null,
  replay: string | null,
): string[] {
  const errors = trailerErrors(message);
  if (agentId !== null && replay === null && !credits(message, agentId)) {
    errors.push(
      `this shell runs agent '${agentId}' but the message has no '${REGISTRY.trailer}: ${agentId}' trailer ` +
        `(lefthook's prepare-commit-msg stamp adds it: keep it in the message, and credit this agent, not another)`,
    );
  }
  return errors;
}

interface Row {
  sha: string;
  date: string;
  c: Classification;
  errors: string[];
}

function rangeRows(range: string): Row[] {
  const out = git([
    'log',
    '--no-merges',
    '--format=%H%x1f%an%x1f%ae%x1f%cI%x1f%B%x1e',
    range,
    '--',
  ]);
  const rows: Row[] = [];
  for (const record of out.split('\x1e')) {
    const fields = record.replace(/^\n/, '').split('\x1f');
    if (fields.length < 5) continue;
    const [sha, name, email, committed, body] = fields;
    const at = new Date(committed);
    rows.push({
      sha,
      date: at.toISOString().slice(0, 10),
      c: classifyCommit({ name, email }, at, body),
      errors: trailerErrors(body),
    });
  }
  return rows;
}

function table(header: string[], rows: string[][]): string {
  const widths = header.map((h, i) =>
    Math.max(h.length, ...rows.map((r) => r[i].length)),
  );
  return [header, ...rows]
    .map((r) =>
      r
        .map((cell, i) => cell.padEnd(widths[i]))
        .join('  ')
        .trimEnd(),
    )
    .join('\n');
}

function perCommit(rows: Row[]): string {
  return table(
    ['sha', 'date', 'kind', 'attribution', 'author', 'assists', 'status'],
    rows.map((r) => [
      r.sha.slice(0, 10),
      r.date,
      r.c.authorKind,
      r.c.attribution,
      r.c.author,
      r.c.assistants
        .map((a) => (a.model ? `${a.agent}:${a.model}` : a.agent))
        .join(',') || '-',
      r.errors.length ? 'INVALID' : 'ok',
    ]),
  );
}

function perAuthor(rows: Row[]): string {
  const groups = new Map<
    string,
    {
      kind: string;
      attr: string;
      commits: number;
      assisted: number;
      by: Map<string, number>;
    }
  >();
  for (const r of rows) {
    const key = `${r.c.author}\x00${r.c.attribution}`;
    const g = groups.get(key) ?? {
      kind: r.c.authorKind,
      attr: r.c.attribution,
      commits: 0,
      assisted: 0,
      by: new Map<string, number>(),
    };
    g.commits += 1;
    if (r.c.assistants.length) g.assisted += 1;
    for (const a of r.c.assistants)
      g.by.set(a.agent, (g.by.get(a.agent) ?? 0) + 1);
    groups.set(key, g);
  }
  const sorted = [...groups.entries()].sort(
    (a, b) => b[1].commits - a[1].commits,
  );
  return table(
    [
      'author',
      'kind',
      'attribution',
      'commits',
      'assisted',
      'assists by agent',
    ],
    sorted.map(([key, g]) => [
      key.split('\x00')[0],
      g.kind,
      g.attr,
      String(g.commits),
      String(g.assisted),
      [...g.by].map(([a, n]) => `${a}=${n}`).join(',') || '-',
    ]),
  );
}

function totals(rows: Row[]): string {
  const count = (pred: (r: Row) => boolean) => rows.filter(pred).length;
  const kinds = (['human', 'agent', 'bot'] as const)
    .map((k) => `${count((r) => r.c.authorKind === k)} ${k}`)
    .join(', ');
  return (
    `${rows.length} non-merge commits: ${kinds}; ` +
    `${count((r) => r.c.assistants.length > 0)} agent-assisted; ` +
    `${count((r) => r.c.attribution === 'legacy')} legacy (before ${REGISTRY.enforcedSince}: no trailer = unknown, not human)`
  );
}

function runRange(range: string, summary: boolean): number {
  let rows: Row[];
  try {
    rows = rangeRows(range);
  } catch (e) {
    console.error(`authorship-check: ${(e as Error).message}`);
    return 2;
  }
  console.log(`authorship-check ${range}`);
  if (rows.length === 0) {
    console.log('no non-merge commits in range');
    return 0;
  }
  console.log(summary ? perAuthor(rows) : perCommit(rows));
  console.log(`\n${totals(rows)}`);
  console.log(
    'note: CI reads trailers only — an agent commit made without the local hooks carries no trailer and is ' +
      "indistinguishable from a human one here; only lefthook's prepare-commit-msg/commit-msg in the agent's shell can detect it.",
  );
  const bad = rows.filter((r) => r.errors.length);
  if (bad.length === 0) return 0;
  console.error(
    `\n${bad.length} commit(s) with invalid ${REGISTRY.trailer} trailers (known agents: ${KNOWN.join(', ')}):`,
  );
  for (const r of bad) {
    for (const e of r.errors) console.error(`  ${r.sha.slice(0, 10)} ${e}`);
  }
  return 1;
}

function runCommitMsg(file: string): number {
  const message = cleanMessage(readFileSync(file, 'utf8'));
  const errors = commitMsgErrors(message, detectAgent(), replaying());
  if (errors.length === 0) return 0;
  console.error('authorship: commit rejected');
  for (const e of errors) console.error(`  - ${e}`);
  console.error(
    `  ${REGISTRY.trailer} format: '<agent-id>[:<model>]', agent-id one of: ${KNOWN.join(', ')} (libs/core/authorship/agents.json)`,
  );
  return 1;
}

if (import.meta.main) {
  const [mode, target, ...rest] = argv.slice(2);
  if (mode === 'commit-msg' && target) exit(runCommitMsg(target));
  if (mode === 'range' && target)
    exit(runRange(target, rest.includes('--summary')));
  console.error(
    'usage: check.ts commit-msg <msg-file> | check.ts range <rev-range> [--summary]',
  );
  exit(2);
}
