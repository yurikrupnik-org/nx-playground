/**
 * Review bundle for the two-pass pre-commit review — `task review-bundle`.
 *
 * Usage: bun tools/review/bundle.ts [--out dist/review/bundle.md]
 *
 * Both review passes (the authoring model, and the independent reviewers /
 * external CLIs) MUST look at the same bytes, and those bytes are the STAGED
 * index, not the working tree and not anyone's memory of the edit. This writes
 * one file containing:
 *
 *   - the staged file table with add/delete counts and a class per file
 *     (generated / test / source / config / docs / binary);
 *   - the repo gates those classes imply, so a diff that touches no Rust never
 *     pays for `task test-rust`;
 *   - generated output, with the command that produces it and the gate that
 *     proves the committed copy matches its source (a required-check list, not
 *     a blocker list — regenerated output is normal to commit);
 *   - what the working tree adds on top, because the external reviewers run
 *     with `--uncommitted` and therefore transmit that too;
 *   - the full staged diff.
 *
 * Hard stops (nothing is written, because the bundle is handed to third-party
 * reviewers): a secret-shaped staged path, or secret-shaped CONTENT in the
 * added lines.
 *
 * Exit codes: 0 bundle written, 1 hard stop (no bundle), 2 nothing staged /
 * bad usage.
 *
 * Contract and the review protocol itself: .claude/skills/precommit-review/SKILL.md
 */

import {
  existsSync,
  lstatSync,
  mkdirSync,
  realpathSync,
  writeFileSync,
} from 'node:fs';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { argv, exit } from 'node:process';

/**
 * Never hand-edited (AGENTS.md): each maps to the command that produces it and
 * the gate that PROVES the committed copy matches its source of truth. A
 * regenerated file in a diff is normal; a hand-edited one is what the proof
 * catches, so this is a required-check list, not a blocker list.
 *
 * A proof must be able to FAIL: `task proto-check` is `cargo check -p rpc`,
 * which compiles the generated tree without comparing it to the .proto, so the
 * proof is regenerate-then-`git diff --exit-code`.
 */
const GENERATED: Record<string, { regen: string; proof: string }> = {
  '**/Tiltfile': { regen: 'task tilt-gen', proof: 'task tilt-check' },
  'manifests/k8s/apps/**': { regen: 'task k8s-gen', proof: 'task k8s-check' },
  '**/k8s/values.yaml': { regen: 'task k8s-gen', proof: 'task k8s-check' },
  '**/k8s/values.*.yaml': { regen: 'task k8s-gen', proof: 'task k8s-check' },
  'libs/*/*/types/**': {
    regen: 'cargo test export_bindings_',
    proof: 'task test-rust + git diff --exit-code',
  },
  'libs/rpc/src/generated/**': {
    regen: 'task proto-gen',
    proof: 'task proto-gen + git diff --exit-code',
  },
  'docs/openapi/**': {
    regen: 'cargo test export_openapi_',
    proof: 'task test-rust + git diff --exit-code',
  },
  '.github/workflows/generated-ci.yml': {
    regen: 'task gen-ci',
    proof: 'task gen-ci + git diff --exit-code',
  },
};

/** Staged paths that must never enter a commit or a review bundle. */
const SECRET_SHAPED = [
  /(^|\/)\.env($|\.)/,
  /(^|\/)\.envrc$/,
  /(^|\/)secrets?\//,
  /\.(pem|key|p8|p12|pfx|tfvars)$/,
  /(^|\/)(id_rsa|id_ed25519|credentials\.json|kubeconfig|\.npmrc)$/,
];

/**
 * Tracked on purpose and holding no value: templates, vals REFERENCES
 * (`ref+gcpsecrets://…`, resolved at run time) and test fixtures. Without this
 * the gate fires on files this repo commits by design — `.env.example`,
 * `manifests/secrets/*.vals.yaml`, `libs/core/oidc-auth/testdata/test_key.pem`.
 */
const SECRET_SHAPED_OK = [
  /\.env\.(example|sample|template)$/,
  /\.vals\.ya?ml$/,
  /(^|\/)testdata\//,
];

/**
 * Secret MATERIAL in added lines. Path rules cannot see a token pasted into an
 * allowed file, and the bundle is transmitted to third-party reviewers.
 */
const SECRET_CONTENT: { name: string; re: RegExp }[] = [
  { name: 'private key block', re: /-----BEGIN [A-Z ]*PRIVATE KEY-----/ },
  { name: 'aws access key id', re: /\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/ },
  { name: 'github token', re: /\bgh[pousr]_[A-Za-z0-9]{36,}\b/ },
  { name: 'slack token', re: /\bxox[abprs]-[A-Za-z0-9-]{10,}\b/ },
  { name: 'google api key', re: /\bAIza[0-9A-Za-z_-]{35}\b/ },
  { name: 'openai/anthropic key', re: /\bsk-(?:ant-)?[A-Za-z0-9_-]{24,}\b/ },
  { name: 'pgp/rsa block', re: /-----BEGIN PGP PRIVATE KEY BLOCK-----/ },
  {
    name: 'inline credential assignment',
    re: /\b(?:api[_-]?key|secret|password|passwd|token)\b\s*[:=]\s*["'][A-Za-z0-9/+_-]{20,}["']/i,
  },
];

/**
 * path -> the gates it implies. Every matching rule contributes, so a diff
 * spanning Rust + butler.toml gets both.
 */
const GATES: { when: RegExp; gates: string[]; why: string }[] = [
  {
    when: /\.rs$|(^|\/)Cargo\.(toml|lock)$/,
    gates: ['task lint-rust', 'task test-rust'],
    why: 'rust sources or the cargo manifests changed',
  },
  {
    when: /^libs\/native\//,
    gates: ['task test-napi'],
    why: 'an N-API addon changed (its targets are JS, not cargo)',
  },
  {
    when: /\.(m?[jt]sx?|cjs|css|astro)$|(^|\/)package\.json$|(^|\/)bun\.lock$/,
    gates: ['task lint-web', 'task test-web'],
    why: 'web sources or JS manifests changed (biome lints .js/.mjs too)',
  },
  {
    when: /(^|\/)butler\.toml$/,
    gates: ['task tilt-check', 'task k8s-check', 'task graph-check'],
    why: 'butler.toml drives the generated Tiltfiles, manifests and inferred targets',
  },
  {
    when: /^tools\/nx\/|(^|\/)project\.json$|^nx\.json$/,
    gates: [
      'task boundaries',
      'task graph-check',
      'task tilt-check',
      'task k8s-check',
    ],
    why: 'nx inference or project tags changed',
  },
  {
    when: /^apps\/butler\/cli\/src\/infer\//,
    gates: ['task graph-check'],
    why: "butler's Rust port of the nx inference changed",
  },
  {
    when: /^manifests\/grpc\/proto\//,
    gates: ['task proto-lint', 'task proto-breaking'],
    why: 'protobuf changed — additivity is a gate',
  },
  {
    when: /^scripts\/kcl\//,
    gates: ['task gen-ci'],
    why: 'the CI workflow is rendered from these KCL sources',
  },
  {
    when: /^docs\/tooling\/registry\.toml$|^tools\/tooling\//,
    gates: ['task tooling-check'],
    why: 'the platform tool registry changed',
  },
  {
    when: /(^|\/)Cargo\.lock$|(^|\/)bun\.lock$/,
    gates: ['task audit', 'task scan'],
    why: 'a lockfile changed — supply-chain scans apply',
  },
  {
    when: /^apps\/todo\/(api|web|web-astro|web-htmx|e2e)\//,
    gates: ['task e2e'],
    why: 'a todo frontend, its backend or the e2e suite changed (docker + Chromium)',
  },
];

let out = 'dist/review/bundle.md';
for (let i = 2; i < argv.length; i++) {
  if (argv[i] === '--out' && argv[i + 1]) out = argv[++i];
  else {
    console.error('usage: bun tools/review/bundle.ts [--out <path>]');
    exit(2);
  }
}

const git = (...args: string[]) => {
  const p = Bun.spawnSync(['git', ...args], { stdout: 'pipe', stderr: 'pipe' });
  if (p.exitCode !== 0) {
    console.error(
      `git ${args.join(' ')} failed: ${p.stderr.toString().trim()}`,
    );
    exit(2);
  }
  return p.stdout.toString();
};

const branch = git('rev-parse', '--abbrev-ref', 'HEAD').trim();
const head = git('rev-parse', '--short', 'HEAD').trim();
// -z, because a rename emits `added\tdeleted\0old\0new` and a path with a
// quote-worthy byte would otherwise come back C-quoted and unusable as a path.
const numstat = git('diff', '--cached', '--numstat', '-z');

if (numstat === '') {
  console.error(
    'review-bundle: nothing staged — `git add` the change under review first',
  );
  exit(2);
}
const root = git('rev-parse', '--show-toplevel').trim();
// `git rev-parse` reports the REAL path (/private/tmp/... on macOS) while
// resolve() keeps the symlinked one, so compare both sides realpath'd — the
// deepest existing ancestor, since the bundle itself may not exist yet.
let ancestor = resolve(out);
const tail: string[] = [];
while (!existsSync(ancestor)) {
  tail.unshift(basename(ancestor));
  const parent = dirname(ancestor);
  if (parent === ancestor) break;
  ancestor = parent;
}
const outPath = join(realpathSync(ancestor), ...tail);
const inRepo = !relative(realpathSync(root), outPath).startsWith('..');
const ignored =
  Bun.spawnSync(['git', 'check-ignore', '-q', outPath]).exitCode === 0;
if (!inRepo || !ignored) {
  // The bundle contains the whole diff. Writing it where git would track it,
  // or outside the repo entirely, is how a diff escapes review into a commit
  // or onto a shared path.
  console.error(
    `review-bundle: --out must be inside the repo and gitignored; \`${out}\` is ${inRepo ? 'not gitignored' : 'outside the repo'}`,
  );
  exit(2);
}
if (existsSync(outPath) && lstatSync(outPath).isSymbolicLink()) {
  console.error(`review-bundle: --out \`${out}\` is a symlink; refusing`);
  exit(2);
}

interface Entry {
  path: string;
  /** `-` for a binary file, which numstat does not count. */
  added: string;
  deleted: string;
  klass: string;
  renamedFrom?: string;
  regen?: string;
  proof?: string;
}

const tokens = numstat.split('\0');
const entries: Entry[] = [];
for (let i = 0; i < tokens.length && tokens[i] !== ''; i++) {
  const [added, deleted, inline] = tokens[i].split('\t');
  // An empty third field means the next two tokens are the rename's old and
  // new paths; review follows the NEW path.
  const renamedFrom = inline === '' ? tokens[++i] : undefined;
  const path = inline === '' ? tokens[++i] : inline;
  const generated = Object.keys(GENERATED).find((g) =>
    new Bun.Glob(g).match(path),
  );
  let klass = 'source';
  if (generated) klass = 'generated';
  else if (added === '-' && deleted === '-') klass = 'binary';
  else if (/\.(md|mdx)$/.test(path)) klass = 'docs';
  else if (
    /(^|\/)(tests?|__test__|e2e)\/|\.(test|spec)\.[tj]sx?$|_it\.rs$/.test(path)
  )
    klass = 'test';
  else if (/\.(toml|json|ya?ml|lock)$/.test(path)) klass = 'config';
  entries.push({
    path,
    added,
    deleted,
    klass,
    renamedFrom,
    regen: generated ? GENERATED[generated].regen : undefined,
    proof: generated ? GENERATED[generated].proof : undefined,
  });
}

const diff = git('diff', '--cached');
const diffLines = diff.split('\n');

const blockers = entries
  .filter(
    (e) =>
      SECRET_SHAPED.some((re) => re.test(e.path)) &&
      !SECRET_SHAPED_OK.some((re) => re.test(e.path)),
  )
  .map(
    (e) =>
      `secret-shaped path staged: \`${e.path}\` — unstage it; nothing secret enters a diff`,
  );

let currentFile = '';
for (const line of diffLines) {
  if (line.startsWith('+++ b/')) currentFile = line.slice(6);
  if (!line.startsWith('+') || line.startsWith('+++')) continue;
  for (const { name, re } of SECRET_CONTENT)
    if (re.test(line))
      blockers.push(
        `${name} in an added line of \`${currentFile}\` — remove it and rotate the credential; nothing secret is sent to a reviewer`,
      );
}

// Nothing is written when a hard stop fires: the bundle is handed to
// third-party reviewers, so materialising the secret to disk first would
// defeat the check it is reporting.
if (blockers.length > 0) {
  console.error(
    `review-bundle: ${blockers.length} hard stop(s) — no bundle written:`,
  );
  for (const b of blockers) console.error(`  ${b}`);
  exit(1);
}

// Regenerated output is a normal thing to commit; a HAND edit is not, and the
// proof is what tells the two apart. Reviewers must not read these as authored
// code.
const generatedEntries = entries.filter((e) => e.klass === 'generated');
const proofs = [...new Set(generatedEntries.map((e) => e.proof))];

// A staged file that is also dirty in the working tree means the reviewers and
// the compiler would be looking at different bytes. The external CLIs review
// `--uncommitted`, so every dirty/untracked file reaches them regardless.
const dirty = git('diff', '--name-only', '-z').split('\0').filter(Boolean);
const untracked = git('ls-files', '--others', '--exclude-standard', '-z')
  .split('\0')
  .filter(Boolean);
const stale = entries.filter((e) => dirty.includes(e.path)).map((e) => e.path);
const extra = [...dirty.filter((p) => !stale.includes(p)), ...untracked];

// `-` is what numstat prints for a binary file; Number('-') is NaN and would
// poison the totals.
const count = (field: 'added' | 'deleted') =>
  entries.reduce((n, e) => n + (e[field] === '-' ? 0 : Number(e[field])), 0);
const binaries = entries.filter((e) => e.klass === 'binary').length;

const gates = GATES.filter((g) => entries.some((e) => g.when.test(e.path)));
const CAP = 4000;
const capped = diffLines.length > CAP;
// A diff of Markdown carries ``` lines that would close a three-backtick
// fence and turn the rest of the bundle into prose for every consumer.
const longestRun = Math.max(
  0,
  ...(diff.match(/`+/g) ?? []).map((run) => run.length),
);
const fence = '`'.repeat(Math.max(3, longestRun + 1));

const body = [
  `# Review bundle — staged diff on \`${branch}\` (HEAD ${head})`,
  '',
  `${entries.length} file(s), +${count('added')} / -${count('deleted')} lines` +
    (binaries > 0 ? `, ${binaries} binary` : '') +
    '.',
  '',
  '## Files',
  '',
  '| file | +/- | class |',
  '|---|---|---|',
  ...entries.map(
    (e) =>
      `| \`${e.path}\`${e.renamedFrom ? ` (renamed from \`${e.renamedFrom}\`)` : ''} | ${e.added === '-' ? 'binary' : `+${e.added}/-${e.deleted}`} | ${e.klass} |`,
  ),
  '',
  '## Gates implied by this diff',
  '',
  gates.length === 0
    ? '_no gate maps to these paths — say so explicitly in the review rather than implying the diff was gated._'
    : gates.map((g) => `- \`${g.gates.join('`, `')}\` — ${g.why}`).join('\n'),
  '',
  generatedEntries.length > 0
    ? `## Generated output in this diff\n\nNot authored code — do not review it line by line; prove it is in sync instead:\n${generatedEntries
        .map((e) => `- \`${e.path}\` ← \`${e.regen}\` (proof: \`${e.proof}\`)`)
        .join('\n')}\n\nRequired here: \`${proofs.join('`, `')}\`\n`
    : '',
  stale.length > 0
    ? `## Stale\n\nStaged but also modified in the working tree — the review would read bytes that are not the ones staged:\n${stale.map((p) => `- \`${p}\``).join('\n')}\n`
    : '',
  extra.length > 0
    ? `## Outside the index\n\nNot staged, but the external reviewers run \`--uncommitted\` and will see these — clean or stash them before pass 2:\n${extra.map((p) => `- \`${p}\``).join('\n')}\n`
    : '',
  '## Staged diff',
  '',
  `${fence}diff`,
  capped ? diffLines.slice(0, CAP).join('\n') : diff,
  fence,
  capped
    ? `\n_diff truncated at ${CAP} of ${diffLines.length} lines — review the remainder with \`git diff --cached -- <path>\` per file._`
    : '',
].join('\n');

mkdirSync(dirname(outPath), { recursive: true });
writeFileSync(outPath, body);

const classes = entries.reduce<Record<string, number>>((acc, e) => {
  acc[e.klass] = (acc[e.klass] ?? 0) + 1;
  return acc;
}, {});
console.log(
  `review-bundle: ${out} — ${entries.length} files (${Object.entries(classes)
    .map(([k, n]) => `${n} ${k}`)
    .join(', ')}), ` +
    `${gates.length} gate group(s)${stale.length > 0 ? `, ${stale.length} stale` : ''}` +
    `${extra.length > 0 ? `, ${extra.length} outside the index` : ''}` +
    `${capped ? ', diff truncated' : ''}`,
);
