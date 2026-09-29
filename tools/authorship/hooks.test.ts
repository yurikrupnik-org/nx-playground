/**
 * The hook entry points as lefthook runs them (`bun stamp.ts <file> <source>`,
 * `bun check.ts commit-msg|range …`) against throwaway git repos, with every
 * agent marker of the registry scrubbed from the inherited environment.
 */

import { afterAll, describe, expect, test } from 'bun:test';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { commitMsgErrors } from './check';
import { REGISTRY } from './registry';

const HERE = import.meta.dir;
const MARKERS = [
  ...REGISTRY.genericEnv,
  ...REGISTRY.agents.flatMap((a) => a.env),
];

/** The parent env minus agent markers and user/system git config. */
function env(extra: Record<string, string> = {}): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(process.env)) {
    if (v !== undefined && !MARKERS.includes(k)) out[k] = v;
  }
  return {
    ...out,
    GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_NAME: 'Test',
    GIT_AUTHOR_EMAIL: 'test@example.com',
    GIT_COMMITTER_NAME: 'Test',
    GIT_COMMITTER_EMAIL: 'test@example.com',
    ...extra,
  };
}

const dirs: string[] = [];
afterAll(() => {
  for (const d of dirs) rmSync(d, { recursive: true, force: true });
});

function repo(): string {
  const dir = mkdtempSync(join(tmpdir(), 'authorship-'));
  dirs.push(dir);
  run(dir, ['git', 'init', '-q']);
  return dir;
}

function run(cwd: string, cmd: string[], extra: Record<string, string> = {}) {
  const r = Bun.spawnSync(cmd, { cwd, env: env(extra), stderr: 'pipe' });
  return {
    code: r.exitCode,
    out: r.stdout.toString(),
    err: r.stderr.toString(),
  };
}

const stamp = (dir: string, file: string, extra: Record<string, string>) =>
  run(dir, ['bun', join(HERE, 'stamp.ts'), file, 'message'], extra);

const checkMsg = (dir: string, file: string, extra: Record<string, string>) =>
  run(dir, ['bun', join(HERE, 'check.ts'), 'commit-msg', file], extra);

function msgFile(dir: string, content: string): string {
  const file = join(dir, 'MSG');
  writeFileSync(file, content);
  return file;
}

describe('stamp.ts (prepare-commit-msg)', () => {
  test('agent shell: one trailer, idempotent across runs', () => {
    const dir = repo();
    const file = msgFile(dir, 'feat: x\n\nbody\n');
    expect(stamp(dir, file, { CLAUDECODE: '1' }).code).toBe(0);
    expect(stamp(dir, file, { CLAUDECODE: '1' }).code).toBe(0);
    expect(readFileSync(file, 'utf8')).toBe(
      'feat: x\n\nbody\n\nAssisted-by: claude-code\n',
    );
  });

  test('an existing credit with a model is left alone', () => {
    const dir = repo();
    const msg = 'feat: x\n\nAssisted-by: claude-code:claude-opus-5\n';
    const file = msgFile(dir, msg);
    stamp(dir, file, { CLAUDECODE: '1' });
    expect(readFileSync(file, 'utf8')).toBe(msg);
  });

  test('human shell and merges are not stamped', () => {
    const dir = repo();
    const file = msgFile(dir, 'feat: x\n');
    stamp(dir, file, {});
    run(dir, ['bun', join(HERE, 'stamp.ts'), file, 'merge'], {
      CLAUDECODE: '1',
    });
    expect(readFileSync(file, 'utf8')).toBe('feat: x\n');
  });

  test('empty editor template keeps line 1 free for the subject', () => {
    const dir = repo();
    const file = msgFile(dir, '\n# Please enter the commit message\n');
    run(dir, ['bun', join(HERE, 'stamp.ts'), file, '{2}'], { OMPCODE: '1' });
    expect(readFileSync(file, 'utf8')).toBe(
      '\n\nAssisted-by: omp\n# Please enter the commit message\n',
    );
  });
});

describe('check.ts commit-msg', () => {
  test('unknown agent id is rejected, even from a human shell', () => {
    const dir = repo();
    const r = checkMsg(dir, msgFile(dir, 'fix: x\n\nAssisted-by: bogus\n'), {});
    expect(r.code).toBe(1);
    expect(r.err).toContain("unknown agent 'bogus'");
  });

  test('agent shell without its trailer is rejected', () => {
    const dir = repo();
    const file = msgFile(dir, 'fix: x\n');
    const r = checkMsg(dir, file, { CODEX_SESSION_ID: 'abc' });
    expect(r.code).toBe(1);
    expect(r.err).toContain("no 'Assisted-by: codex' trailer");
    expect(checkMsg(dir, file, {}).code).toBe(0);
  });

  test('comment lines are ignored, as git strips them', () => {
    const dir = repo();
    const file = msgFile(
      dir,
      'fix: x\n\nAssisted-by: codex\n\n# Please enter the commit message\n',
    );
    expect(checkMsg(dir, file, { CODEX_SESSION_ID: 'abc' }).code).toBe(0);
  });

  test('value format', () => {
    expect(
      commitMsgErrors('fix: x\n\nAssisted-by: omp:opus', null, null),
    ).toEqual([]);
    expect(
      commitMsgErrors('fix: x\n\nAssisted-by: omp:', null, null),
    ).toHaveLength(1);
    expect(
      commitMsgErrors('fix: x\n\nAssisted-by: Claude Code', null, null),
    ).toHaveLength(1);
  });

  test('replays are exempt from crediting, not from the value check', () => {
    expect(commitMsgErrors('fix: x', 'omp', 'rebase')).toEqual([]);
    expect(
      commitMsgErrors('fix: x\n\nAssisted-by: nope', 'omp', 'merge'),
    ).toHaveLength(1);
  });
});

describe('check.ts range', () => {
  test('fails on an invalid trailer anywhere in the range', () => {
    const dir = repo();
    const commit = (msg: string) =>
      run(dir, ['git', 'commit', '-q', '--allow-empty', '-m', msg]);
    commit('chore: root');
    commit('feat: ok\n\nAssisted-by: codex');
    const ok = run(dir, ['bun', join(HERE, 'check.ts'), 'range', 'HEAD']);
    expect(ok.code).toBe(0);
    expect(ok.out).toContain('codex');

    commit('feat: bad\n\nAssisted-by: bogus');
    const bad = run(dir, [
      'bun',
      join(HERE, 'check.ts'),
      'range',
      'HEAD~1..HEAD',
    ]);
    expect(bad.code).toBe(1);
    expect(bad.err).toContain("unknown agent 'bogus'");
  });
});
