/**
 * The little git plumbing the authorship hooks share: running git, reading a
 * commit message the way git will store it, and telling a fresh commit from a
 * replayed one.
 */

import { existsSync } from 'node:fs';
import { join } from 'node:path';

/** Run git; returns stdout, throws with stderr on a non-zero exit. */
export function git(args: string[], cwd?: string): string {
  const r = Bun.spawnSync(['git', ...args], { cwd, stderr: 'pipe' });
  if (r.exitCode !== 0) {
    throw new Error(
      `git ${args.join(' ')}: ${r.stderr.toString().trim() || `exit ${r.exitCode}`}`,
    );
  }
  return r.stdout.toString();
}

/**
 * The message as git will commit it (default `--cleanup=strip`): everything
 * below the `git commit -v` scissors line and every comment line dropped. The
 * commit-msg / prepare-commit-msg hooks still see the editor template, the
 * trailer rules must not.
 */
export function cleanMessage(raw: string, cwd?: string): string {
  let comment = '#';
  try {
    const c = git(['config', '--get', 'core.commentChar'], cwd).trim();
    if (c !== '' && c !== 'auto') comment = c;
  } catch {
    // unset: git's default
  }
  const kept: string[] = [];
  for (const line of raw.split('\n')) {
    if (
      line === `${comment} ------------------------ >8 ------------------------`
    )
      break;
    if (!line.startsWith(comment)) kept.push(line);
  }
  return kept.join('\n');
}

/**
 * Why the commit being made is a replay of existing work — a merge, or a
 * rebase / cherry-pick re-creating commits that keep their original
 * attribution — or `null` for a fresh commit.
 */
export function replaying(cwd?: string): string | null {
  const dir = git(['rev-parse', '--absolute-git-dir'], cwd).trim();
  if (existsSync(join(dir, 'MERGE_HEAD'))) return 'merge';
  if (
    existsSync(join(dir, 'rebase-merge')) ||
    existsSync(join(dir, 'rebase-apply'))
  )
    return 'rebase';
  if (existsSync(join(dir, 'CHERRY_PICK_HEAD'))) return 'cherry-pick';
  return null;
}
