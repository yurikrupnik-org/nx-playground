/**
 * Inferred cargo targets for one crate: `build`, `test` and `doc`, plus
 * `doc-test` for a crate with a library, `run` for one that has a binary and
 * `install` for one that is `publish`able. `lint` is next door in
 * `polyglot-targets.ts`, which composes clippy with biome for the crates that
 * are also TS packages.
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
 * same work as 90 nx tasks. The per-crate `test` target here — and the `lint`
 * one next door — are for the other scope, the one cargo cannot express: what a
 * diff actually touched, with Nx Cloud caching a crate's clippy/nextest result
 * across CI runs (1 crate, both targets, 2/2 cache hits: 4s).
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
 * Does a package.json script already own this crate's `lint`/`test`? nx's package.json
 * inference wins over a plugin target, so for the N-API addons — whose
 * deliverable is a JS package and whose `test` is vitest — the cargo gate never
 * runs. Tagging them `rust` would hand a vitest run to
 * `just check-rust-affected`, whose contract is clippy + nextest; they are
 * covered by `just test-napi` and, for clippy, by the workspace
 * `just lint-rust` (and by their own composed `lint` target).
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

export function rustTargets(
  crate: CargoCrate,
  dir: string,
): Record<string, unknown> {
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
    // `--no-tests=pass`: nextest exits 4 ("no tests to run") on a crate with no
    // test binaries, which is a normal state for a crate in isolation (and the
    // permanent state of the cdylib N-API addons, `[lib] test = false`) even
    // though `--workspace` always finds some.
    test: {
      executor: 'nx:run-commands',
      cache: true,
      // What invalidates a cargo gate: the crate's own files, the files of the
      // crates it depends on (`^default`, resolved over `@monodon/rust`'s cargo
      // dependency edges) and the workspace-level knobs that change how every
      // crate compiles — `rustGlobals` in `nx.json`.
      inputs: ['default', '^default', 'rustGlobals'],
      // NO outputs: cargo writes into the shared `dist/target`, which nx cannot
      // fingerprint per crate, so an entry means "these inputs passed nextest",
      // never "an artifact was restored". `nx.json`'s `test` targetDefault sets
      // only `cache`, so these inputs survive — the `build` default owns
      // `inputs`/`outputs` for its name, which is why `build` is spelled out.
      outputs: [],
      options: {
        command: `cargo nextest run --package ${crate.name} --no-tests=pass`,
        cwd: '{workspaceRoot}',
      },
      metadata: {
        description: `cargo nextest run ${crate.name}`,
        technologies: ['rust'],
      },
    },
    // rustdoc is a THIRD compiler front-end over the same sources: it resolves
    // every intra-doc link and every `#[doc]` attribute, which neither clippy
    // nor nextest does. `-D warnings` is what turns that into a gate — without
    // it a broken link is a warning nobody reads, and `monodocs build
    // --cargo-doc` (the docs site's API section) ships the hole.
    // `--no-deps`: the dependency docs are not this crate's to gate.
    // Env inline rather than `options.env` so the command is the whole truth —
    // `just doc-check` and butler's runner both execute the string verbatim.
    doc: {
      executor: 'nx:run-commands',
      cache: true,
      inputs: ['default', '^default', 'rustGlobals'],
      // Same reasoning as `test`: rustdoc's HTML lands in the shared
      // `dist/target/doc`, so a cache entry means "these inputs documented
      // cleanly", not "an artifact was restored".
      outputs: [],
      options: {
        command: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --package ${crate.name}`,
        cwd: '{workspaceRoot}',
      },
      metadata: {
        description: `cargo doc ${crate.name} (warnings are errors)`,
        technologies: ['rust'],
      },
    },
  };

  // nextest CANNOT run doctests (it has no rustdoc harness), so `test` above
  // leaves every `/// ```` example in this workspace uncompiled — 210 of them
  // across 9 crates before this target existed. Only a `[lib]` gets one:
  // `cargo test --doc` on a bin-only package fails with "no library targets
  // found", which would be a red target for a crate that simply has no
  // doctests to run.
  if (crate.hasLibrary) {
    targets['doc-test'] = {
      executor: 'nx:run-commands',
      cache: true,
      inputs: ['default', '^default', 'rustGlobals'],
      outputs: [],
      options: {
        command: `cargo test --doc --package ${crate.name}`,
        cwd: '{workspaceRoot}',
      },
      metadata: {
        description: `cargo test --doc ${crate.name}`,
        technologies: ['rust'],
      },
    };
  }

  if (!crate.hasBinary) return targets;

  // `cargo install` for a crate whose binary is meant to leave this repo:
  // `[package] publish`, today only `butler`, the CLI every k8s/tilt target and
  // half the just recipes shell out to. NOT cached: the artifact lands in
  // `~/.cargo/bin`, outside anything nx fingerprints, so a cache hit would
  // report success while the machine still has the old binary — or none at
  // all, after a `rm ~/.cargo/bin/<bin>`.
  //
  // `--force` because without it cargo REFUSES to overwrite an install of the
  // same version, which is the normal state here: the version only moves when
  // the release workflow bumps it, while the source moves every commit.
  // `--locked` pins the committed Cargo.lock, so the installed binary is built
  // from the dependency set CI resolved, not a fresh one.
  const install: Record<string, unknown> = crate.publish
    ? {
        install: {
          executor: 'nx:run-commands',
          cache: false,
          options: {
            command: `cargo install --path ${dir} --locked --force`,
            cwd: '{workspaceRoot}',
          },
          metadata: {
            description: `cargo install ${crate.name} into ~/.cargo/bin`,
            technologies: ['rust'],
          },
        },
      }
    : {};

  return {
    ...targets,
    ...install,
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
