#!/usr/bin/env bun
/**
 * lefthook `prepare-commit-msg`: credit the AI coding agent whose shell is
 * making the commit — `bun tools/authorship/stamp.ts <msg-file> [<source>]`.
 *
 * - Human shell (no marker from libs/core/authorship/agents.json set): no-op.
 * - Agent shell: appends `Assisted-by: <agent-id>` unless the message already
 *   credits that agent (any `Assisted-by: <id>[:<model>]` or a registered
 *   `Co-authored-by:` email), via `git interpret-trailers --if-exists
 *   addIfDifferent`, so running twice leaves one trailer.
 * - Only the agent id is written: no agent exports its model name in a
 *   documented variable. An agent (or person) may write `<id>:<model>` itself;
 *   the check accepts it and the stamp then leaves it alone.
 * - Skipped: `<source>` = `merge` and any replay in progress (rebase,
 *   cherry-pick, merge with MERGE_HEAD) — merges are excluded from attribution
 *   and replayed commits keep the credit they were first made with.
 *   `squash` (`git merge --squash` + commit) and `commit` (`--amend`, `-c`)
 *   create a new non-merge commit in this shell, so they ARE stamped.
 *
 * lefthook leaves a placeholder it has no argument for as literal text
 * (`{2}`), which counts as absent here.
 */

import { readFileSync, writeFileSync } from 'node:fs';
import { argv, exit } from 'node:process';
import { cleanMessage, git, replaying } from './git';
import { credits, detectAgent, REGISTRY } from './registry';

const [file, rawSource] = argv
  .slice(2)
  .map((a) => (/^\{\d\}$/.test(a) || a === '' ? undefined : a));
if (!file) {
  console.error('usage: stamp.ts <commit-msg-file> [<source> [<sha>]]');
  exit(2);
}

const id = detectAgent();
if (id === null || rawSource === 'merge' || replaying() !== null) exit(0);

const raw = readFileSync(file, 'utf8');
const message = cleanMessage(raw);
if (credits(message, id)) exit(0);

const trailer = `${REGISTRY.trailer}: ${id}`;
if (message.trim() === '') {
  // Editor commit from the empty template: keep line 1 free for the subject
  // and a blank line before the trailer, as `git commit -s` does.
  writeFileSync(file, `\n\n${trailer}\n${raw.replace(/^\n+/, '')}`);
} else {
  git([
    'interpret-trailers',
    '--in-place',
    '--if-exists',
    'addIfDifferent',
    '--trailer',
    trailer,
    file,
  ]);
}
