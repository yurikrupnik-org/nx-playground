/**
 * Parity with libs/core/authorship/src/lib.rs: the cases mirror its unit
 * tests, so a rule changed on one side only fails here or there.
 */

import { describe, expect, test } from 'bun:test';
import {
  assists,
  classifyCommit,
  credits,
  detectAgent,
  parseTrailers,
  REGISTRY,
} from './registry';

const human = {
  login: 'yurikrupnik',
  name: 'Yuri Krupnik',
  email: 'krupnik.yuri@gmail.com',
};
const at = (y: number, m: number, d: number) =>
  new Date(Date.UTC(y, m - 1, d, 12));

describe('detectAgent', () => {
  // omp exports CLAUDECODE as well; the more specific marker must win.
  test('follows registry order', () => {
    expect(detectAgent({ CLAUDECODE: '1', OMPCODE: '1', AGENT: '1' })).toBe(
      'omp',
    );
    expect(detectAgent({ CLAUDECODE: '1' })).toBe('claude-code');
    expect(detectAgent({ CODEX_SANDBOX: 'seatbelt' })).toBe('codex');
  });

  test('generic markers and falsy values', () => {
    expect(detectAgent({ AGENT: '1' })).toBe('unknown-agent');
    expect(detectAgent({ CLAUDECODE: '0' })).toBeNull();
    expect(detectAgent({ GEMINI_CLI: '' })).toBeNull();
    expect(detectAgent({ CURSOR_AGENT: 'false' })).toBeNull();
    expect(detectAgent({ CURSOR_AGENT: ' FALSE ' })).toBeNull();
    expect(detectAgent({})).toBeNull();
  });

  test('a falsy specific marker does not hide a truthy later one', () => {
    expect(detectAgent({ OMPCODE: '0', CLAUDECODE: '1' })).toBe('claude-code');
  });
});

describe('parseTrailers', () => {
  test('come from the last paragraph only', () => {
    const msg =
      'feat(x): thing\n\nBody mentions Key: value in prose.\n\nAssisted-by: claude-code:claude-opus-5\nSigned-off-by: A <a@b>\n';
    const t = parseTrailers(msg);
    expect(t).toHaveLength(2);
    expect(t[0]).toEqual({
      key: 'Assisted-by',
      value: 'claude-code:claude-opus-5',
    });
    expect(parseTrailers('fix: subject only')).toEqual([]);
    expect(
      parseTrailers(
        'fix: x\n\nJust a closing sentence: really.\nAnd more prose',
      ),
    ).toEqual([]);
  });

  test('split at the rightmost blank line, like Rust rsplit', () => {
    expect(parseTrailers('fix: x\n\n\nAssisted-by: omp')).toEqual([
      { key: 'Assisted-by', value: 'omp' },
    ]);
  });

  test('continuation lines, CRLF and malformed keys', () => {
    expect(
      parseTrailers('fix: x\n\nNote: first\n  second\r\nAssisted-by: omp\r\n'),
    ).toEqual([
      { key: 'Note', value: 'first second' },
      { key: 'Assisted-by', value: 'omp' },
    ]);
    expect(parseTrailers('fix: x\n\n  leading: continuation')).toEqual([]);
    expect(parseTrailers('fix: x\n\nAssisted by: omp')).toEqual([]);
    expect(parseTrailers('fix: x\n\nAssisted-by:   ')).toEqual([]);
  });
});

describe('assists', () => {
  test('merge Assisted-by and known co-authors, first mention wins', () => {
    const msg =
      'feat: x\n\nAssisted-by: omp:claude-opus-5\nCo-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>\nCo-authored-by: Jane <jane@example.com>\nAssisted-by: omp\n';
    expect(assists(msg)).toEqual([
      { agent: 'omp', model: 'claude-opus-5' },
      { agent: 'claude-code', model: null },
    ]);
  });

  test('the generic id is satisfied by any registered agent', () => {
    expect(credits('fix: x\n\nAssisted-by: codex', 'unknown-agent')).toBe(true);
    expect(credits('fix: x\n\nAssisted-by: codex', 'omp')).toBe(false);
  });
});

describe('classifyCommit', () => {
  test('agent identities are agents, not bots', () => {
    const c = classifyCommit(
      {
        login: 'claude-maintenance[bot]',
        name: 'claude-maintenance[bot]',
        email: 'claude-maintenance[bot]@users.noreply.github.com',
      },
      at(2026, 10, 1),
      'chore: bump\n\nAssisted-by: claude-code\n',
    );
    expect(c.author).toBe('claude-code');
    expect(c.authorKind).toBe('agent');
    expect(c.assistants).toEqual([]);
  });

  test('unregistered bot identities are bots', () => {
    const c = classifyCommit(
      {
        name: 'github-actions[bot]',
        email: 'github-actions[bot]@users.noreply.github.com',
      },
      at(2026, 10, 1),
      'chore(release): v1',
    );
    expect(c.author).toBe('github-actions[bot]');
    expect(c.authorKind).toBe('bot');
  });

  test('humans keep their kind and gain assistants', () => {
    const c = classifyCommit(
      human,
      at(2026, 10, 1),
      'feat: x\n\nAssisted-by: codex\n',
    );
    expect(c.author).toBe('yurikrupnik');
    expect(c.authorKind).toBe('human');
    expect(c.assistants.map((a) => a.agent)).toEqual(['codex']);
    expect(c.attribution).toBe('enforced');

    const noLogin = classifyCommit(
      { name: 'Slava', email: 'Slava@Example.com' },
      at(2026, 1, 1),
      'fix: y',
    );
    expect(noLogin.author).toBe('slava@example.com');
    expect(noLogin.attribution).toBe('legacy');
  });

  test('enforcement starts on the registry date (UTC)', () => {
    const on = new Date(`${REGISTRY.enforcedSince}T00:00:00Z`);
    const before = new Date(on.getTime() - 1000);
    expect(classifyCommit(human, on, 'x').attribution).toBe('enforced');
    expect(classifyCommit(human, before, 'x').attribution).toBe('legacy');
  });
});
