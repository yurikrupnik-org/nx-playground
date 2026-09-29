/**
 * TypeScript mirror of `libs/core/authorship` (crate `core_authorship`): the
 * git hooks (`stamp.ts`, `check.ts commit-msg`) and the CI range check read
 * the same `agents.json` registry as the Rust classifiers, and every function
 * here follows its Rust namesake exactly — change both together.
 *
 * - `detectAgent`  ↔ `Registry::detect_agent`
 * - `parseTrailers` ↔ `trailers`
 * - `assists`       ↔ `Registry::assists`
 * - `classifyCommit` ↔ `classify_commit`
 */

import registryJson from '../../libs/core/authorship/agents.json' with {
  type: 'json',
};

export interface Agent {
  id: string;
  name: string;
  /** Environment variables the agent exports into the shells it spawns. */
  env: string[];
  /** Git author names / GitHub logins the agent commits as. */
  identities: string[];
  /** Emails the agent's `Co-authored-by:` trailers carry. */
  coauthorEmails: string[];
}

export interface Registry {
  /** Trailer key the hooks stamp (`Assisted-by`). */
  trailer: string;
  /** First UTC day (`YYYY-MM-DD`) on which agent commits are stamped. */
  enforcedSince: string;
  /** Markers that say "some agent" without saying which. */
  genericEnv: string[];
  /** Agent id reported for `genericEnv` matches. */
  genericAgentId: string;
  /** Detection order: the first agent with a marker set wins. */
  agents: Agent[];
}

export const REGISTRY: Registry = registryJson;

export type Env = Record<string, string | undefined>;

/** Rust `to_ascii_lowercase`: only A–Z change. */
const asciiLower = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

const eqIgnoreAsciiCase = (a: string, b: string) =>
  asciiLower(a) === asciiLower(b);

/** A marker counts when it is set to anything but empty, `0` or `false`. */
export function isTruthy(value: string): boolean {
  const v = value.trim();
  return !(v === '' || v === '0' || eqIgnoreAsciiCase(v, 'false'));
}

export function agent(id: string, reg: Registry = REGISTRY): Agent | undefined {
  return reg.agents.find((a) => a.id === id);
}

/** Whether `id` is a registered agent or the generic fallback id. */
export function isKnownAgent(id: string, reg: Registry = REGISTRY): boolean {
  return id === reg.genericAgentId || agent(id, reg) !== undefined;
}

/**
 * The agent whose markers are set in `env`, in registry order; the generic id
 * when only a generic marker is set; `null` in a human shell.
 */
export function detectAgent(
  env: Env = process.env,
  reg: Registry = REGISTRY,
): string | null {
  const set = (name: string) => {
    const v = env[name];
    return v !== undefined && isTruthy(v);
  };
  const found = reg.agents.find((a) => a.env.some(set));
  if (found) return found.id;
  return reg.genericEnv.some(set) ? reg.genericAgentId : null;
}

/** The agent that commits as `identity` (a git author name or GitHub login). */
export function agentForIdentity(
  identity: string,
  reg: Registry = REGISTRY,
): Agent | undefined {
  return reg.agents.find((a) =>
    a.identities.some((i) => eqIgnoreAsciiCase(i, identity.trim())),
  );
}

/** The agent whose `Co-authored-by:` trailers carry `email`. */
export function agentForEmail(
  email: string,
  reg: Registry = REGISTRY,
): Agent | undefined {
  return reg.agents.find((a) =>
    a.coauthorEmails.some((e) => eqIgnoreAsciiCase(e, email.trim())),
  );
}

export interface Trailer {
  key: string;
  value: string;
}

const TRAILER_KEY = /^[A-Za-z0-9-]+$/;

/**
 * The trailers of a commit message: its last paragraph, when every line of it
 * is `Key: value` (a line starting with whitespace continues the previous
 * value). A last paragraph that is prose yields nothing.
 */
export function parseTrailers(message: string): Trailer[] {
  const body = message.trimEnd();
  // Rust `rsplit("\n\n").next()`: the text after the RIGHTMOST separator.
  const cut = body.lastIndexOf('\n\n');
  const last = cut === -1 ? body : body.slice(cut + 2);
  // A subject line alone is not a trailer block.
  if (last.length === body.length && !body.includes('\n')) return [];
  const out: Trailer[] = [];
  // Rust `str::lines`: split on \n, drop one trailing \r per line.
  for (const raw of last.split('\n')) {
    const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
    if (line.startsWith(' ') || line.startsWith('\t')) {
      const prev = out.at(-1);
      if (!prev) return [];
      prev.value += ` ${line.trim()}`;
      continue;
    }
    const colon = line.indexOf(':');
    if (colon === -1) return [];
    const key = line.slice(0, colon);
    const value = line.slice(colon + 1).trim();
    if (!TRAILER_KEY.test(key) || value === '') return [];
    out.push({ key, value });
  }
  return out;
}

export interface Assist {
  /** Registry id when recognised; otherwise the value as written, lowercased. */
  agent: string;
  /** `Assisted-by: <agent>:<model>` carries the model after the colon. */
  model: string | null;
}

/** The `<email>` part of `Name <email>`. */
function trailerEmail(value: string): string | null {
  const start = value.lastIndexOf('<');
  if (start === -1) return null;
  const end = value.indexOf('>', start);
  if (end === -1) return null;
  return value.slice(start + 1, end).trim();
}

/** Split an `Assisted-by` value into `<agent>` and optional `<model>`. */
export function splitAssist(value: string): {
  agent: string;
  model: string | null;
} {
  const colon = value.indexOf(':');
  if (colon === -1) return { agent: value, model: null };
  const model = value.slice(colon + 1).trim();
  return { agent: value.slice(0, colon), model: model === '' ? null : model };
}

/**
 * Agents credited by `message`: every `Assisted-by:` trailer, plus every
 * `Co-authored-by:` whose email belongs to a registered agent. First mention
 * wins; an agent is listed once.
 */
export function assists(message: string, reg: Registry = REGISTRY): Assist[] {
  const out: Assist[] = [];
  const push = (a: Assist) => {
    if (!out.some((o) => o.agent === a.agent)) out.push(a);
  };
  for (const t of parseTrailers(message)) {
    if (eqIgnoreAsciiCase(t.key, reg.trailer)) {
      const { agent: id, model } = splitAssist(t.value);
      push({ agent: asciiLower(id.trim()), model });
    } else if (eqIgnoreAsciiCase(t.key, 'Co-authored-by')) {
      const email = trailerEmail(t.value);
      const a = email === null ? undefined : agentForEmail(email, reg);
      if (a) push({ agent: a.id, model: null });
    }
  }
  return out;
}

/**
 * Whether `message` already credits agent `id` — the rule both hooks share.
 * The generic id (`AGENT=1`, agent unknown) is satisfied by any registered
 * agent the author credited explicitly.
 */
export function credits(
  message: string,
  id: string,
  reg: Registry = REGISTRY,
): boolean {
  return assists(message, reg).some(
    (a) =>
      a.agent === id ||
      (id === reg.genericAgentId && isKnownAgent(a.agent, reg)),
  );
}

export type ContributorKind = 'human' | 'agent' | 'bot';
export type Attribution = 'enforced' | 'legacy';

/** A commit's author as git and GitHub report it. */
export interface CommitAuthor {
  /** GitHub login, when the email is linked to an account (git alone: none). */
  login?: string;
  name: string;
  email: string;
}

export interface Classification {
  /** Agent id for an agent identity, the GitHub login when known, else the lowercased email. */
  author: string;
  authorKind: ContributorKind;
  /** Agents credited alongside the author, never the author itself. */
  assistants: Assist[];
  attribution: Attribution;
}

/** GitHub's noreply domain for app/bot accounts. */
const BOT_EMAIL_SUFFIX = '[bot]@users.noreply.github.com';

/**
 * Classify one commit. Precedence: an identity registered to an agent is that
 * agent; any other `[bot]` identity is a bot; everyone else is a human.
 * Trailers never change the author's kind — they add assistants. Attribution
 * compares the UTC commit date with `enforcedSince`.
 */
export function classifyCommit(
  author: CommitAuthor,
  committedAt: Date,
  message: string,
  reg: Registry = REGISTRY,
): Classification {
  const a =
    (author.login === undefined
      ? undefined
      : agentForIdentity(author.login, reg)) ??
    agentForIdentity(author.name, reg) ??
    agentForIdentity(author.email, reg);
  let id: string;
  let kind: ContributorKind;
  if (a) {
    id = a.id;
    kind = 'agent';
  } else if (
    (author.login?.endsWith('[bot]') ?? false) ||
    author.name.endsWith('[bot]') ||
    asciiLower(author.email).endsWith(BOT_EMAIL_SUFFIX)
  ) {
    id = author.login?.endsWith('[bot]') ? author.login : author.name;
    kind = 'bot';
  } else {
    id = author.login ?? asciiLower(author.email.trim());
    kind = 'human';
  }
  const day = committedAt.toISOString().slice(0, 10);
  return {
    author: id,
    authorKind: kind,
    assistants: assists(message, reg).filter((x) => x.agent !== id),
    attribution: day >= reg.enforcedSince ? 'enforced' : 'legacy',
  };
}
