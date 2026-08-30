/**
 * Inferred cargo `build` / `run` targets for one crate.
 *
 * `@monodon/rust` contributes graph nodes, cargo dependency edges and exactly
 * one target (`nx-release-publish`) — its source still carries a
 * `TODO: provide defaults for non-project.json workspaces`. Every crate in this
 * repo therefore used to hand-copy the same two blocks into its `project.json`,
 * ~30 files whose only variable was the package name; they had already drifted
 * (some apps carried a `production` configuration, some did not). This module is
 * that boilerplate, derived from the one file that cannot lie about a crate's
 * name: its `Cargo.toml`.
 *
 * These targets exist for graph and CI shape — `nx affected`, `nx run <crate>:run`
 * on a single service — NOT for building the workspace. Running Rust through nx
 * is 3-8x slower (per-crate cargo processes serialize on the target-dir lock and
 * the shared `dist/target` is not fingerprintable), which is why `just test-rust`
 * / `just lint-rust` stay cargo-direct and why no `test` or `lint` target is
 * inferred here: a target that exists is a target someone will run.
 *
 * `plugin.ts` owns the nx registration; this file is imported into that single
 * plugin worker rather than registered as a plugin of its own.
 */

import type { CargoCrate } from './butler-config.ts';

/**
 * Every crate manifest below a first-level directory, which excludes the
 * workspace root's own virtual manifest — the same glob `@monodon/rust` uses to
 * find these files.
 */
export const CARGO_MANIFESTS = '*/**/Cargo.toml';

export function rustTargets(crate: CargoCrate): Record<string, unknown> {
  // A library has no artifact worth linking, so `check` is the cheap answer to
  // "does this still compile"; a binary crate is built for real.
  const verb = crate.hasBinary ? 'build' : 'check';
  const build = {
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
  };
  if (!crate.hasBinary) return { build };

  return {
    build: {
      ...build,
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
