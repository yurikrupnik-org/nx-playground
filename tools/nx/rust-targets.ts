/**
 * Inferred cargo targets for one crate: `build`, `lint`, `test`, plus `run` for
 * a crate that has a binary.
 *
 * `@monodon/rust` contributes graph nodes, cargo dependency edges and exactly
 * one target (`nx-release-publish`) — its source still carries a
 * `TODO: provide defaults for non-project.json workspaces`. Every crate in this
 * repo therefore used to hand-copy the same blocks into its `project.json`,
 * ~30 files whose only variable was the package name; they had already drifted
 * (some apps carried a `production` configuration, some did not). This module is
 * that boilerplate, derived from the one file that cannot lie about a crate's
 * name: its `Cargo.toml`.
 *
 * Two orchestrators, two jobs. `just lint-rust` / `just test-rust` stay
 * cargo-direct (`cargo clippy --workspace`, `cargo nextest run --workspace`) and
 * remain the authoritative gate: ONE cargo process shares the target-dir lock
 * and compiles each shared dependency once. Measured warm on 45 crates: 2m13s
 * that way, against 4m06s (`--parallel=4`) and 10m09s (`--parallel=1`) for the
 * same work as 90 nx tasks. The per-crate `lint`/`test` targets here are for the
 * other scope — the one cargo cannot express: what a diff actually touched, with
 * Nx Cloud caching a crate's clippy/nextest result across CI runs (1 crate, both
 * targets, 2/2 cache hits: 4s).
 *
 * They have exactly one caller, `just check-rust-affected`, which also owns the
 * cutover back: past ~20 affected crates it drops these targets and runs the
 * workspace gate instead. Do not invoke them over the whole graph
 * (`nx run-many -t lint test -p tag:rust` is the 10-minute path by construction).
 *
 * `plugin.ts` owns the nx registration; this file is imported into that single
 * plugin worker rather than registered as a plugin of its own.
 */

import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import type { CargoCrate } from './butler-config.ts';

/**
 * Every crate manifest below a first-level directory, which excludes the
 * workspace root's own virtual manifest — the same glob `@monodon/rust` uses to
 * find these files.
 */
export const CARGO_MANIFESTS = '*/**/Cargo.toml';

/**
 * Tag on every crate node, so a gate can address the Rust half of the graph:
 * `nx affected -t lint test -p tag:rust`. Without the filter that command would
 * also fire the web `lint` scripts, which are mutating `biome check --write`
 * (see AGENTS.md) and therefore not gates.
 */
export const RUST_TAG = 'rust';

/**
 * Does a package.json script already own this crate's `lint`/`test`? nx's
 * package.json inference wins over a plugin target, so for the N-API addons —
 * whose deliverable is a JS package (`lint` is a MUTATING
 * `biome check --write`, `test` is vitest) — the cargo gate below never runs.
 * Tagging them `rust` would drag that mutating lint into
 * `nx affected -t lint -p tag:rust`; they are covered by `just test-napi` and,
 * for clippy, by the workspace `just lint-rust`.
 */
export function hasPackageScriptGates(
  workspaceRoot: string,
  dir: string,
): boolean {
  const path = join(workspaceRoot, dir, 'package.json');
  if (!existsSync(path)) return false;
  const scripts = (
    JSON.parse(readFileSync(path, 'utf8')) as {
      scripts?: Record<string, string>;
    }
  ).scripts;
  return scripts !== undefined && ('lint' in scripts || 'test' in scripts);
}

/**
 * What invalidates a cargo target: the crate's own files, the files of the
 * crates it depends on (`^default`, resolved over `@monodon/rust`'s cargo
 * dependency edges) and the workspace-level knobs that change how every crate
 * compiles — `rustGlobals` in `nx.json`.
 */
const INPUTS = ['default', '^default', 'rustGlobals'];

/**
 * A cached gate (`lint`, `test`): cargo writes into the shared `dist/target`,
 * which nx cannot fingerprint per crate, so declaring NO outputs is what keeps
 * the cache honest — an entry means "these inputs passed clippy/nextest", never
 * "an artifact was restored". `nx.json`'s `targetDefaults` set only `cache` for
 * these two names, so the inputs below survive; the `build` default owns
 * `inputs`/`outputs` for its name and would overwrite anything set here, which
 * is why `build` is spelled out separately.
 */
function cargoGate(
  command: string,
  description: string,
): Record<string, unknown> {
  return {
    executor: 'nx:run-commands',
    cache: true,
    inputs: INPUTS,
    outputs: [],
    options: { command, cwd: '{workspaceRoot}' },
    metadata: { description, technologies: ['rust'] },
  };
}

export function rustTargets(crate: CargoCrate): Record<string, unknown> {
  // A library has no artifact worth linking, so `check` is the cheap answer to
  // "does this still compile"; a binary crate is built for real.
  const verb = crate.hasBinary ? 'build' : 'check';
  const targets: Record<string, unknown> = {
    // `inputs`/`outputs` deliberately absent: the `build` targetDefault wins
    // over an inferred target, so anything set here would be dropped silently.
    build: {
      executor: 'nx:run-commands',
      cache: true,
      options: {
        command: `cargo ${verb} --package ${crate.name}`,
        cwd: '{workspaceRoot}',
      },
      metadata: {
        description: `cargo ${verb} ${crate.name}`,
        technologies: ['rust'],
      },
    },
    // Same flags as `just lint-rust`, one crate at a time.
    lint: cargoGate(
      `cargo clippy --package ${crate.name} --all-targets -- -D warnings`,
      `cargo clippy ${crate.name}`,
    ),
    // `--no-tests=pass`: nextest exits 4 ("no tests to run") on a crate with no
    // test binaries, which is a normal state for a crate in isolation (and the
    // permanent state of the cdylib N-API addons, `[lib] test = false`) even
    // though `--workspace` always finds some.
    test: cargoGate(
      `cargo nextest run --package ${crate.name} --no-tests=pass`,
      `cargo nextest run ${crate.name}`,
    ),
  };

  if (!crate.hasBinary) return targets;

  return {
    ...targets,
    build: {
      ...(targets.build as Record<string, unknown>),
      configurations: {
        production: {
          command: `cargo build --package ${crate.name} --release`,
        },
      },
    },
    run: {
      executor: 'nx:run-commands',
      options: {
        command: `cargo run --package ${crate.name}`,
        cwd: '{workspaceRoot}',
      },
      configurations: {
        production: { command: `cargo run --package ${crate.name} --release` },
      },
      metadata: {
        description: `cargo run ${crate.name}`,
        technologies: ['rust'],
      },
    },
  };
}
