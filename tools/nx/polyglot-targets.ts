/**
 * The two target names BOTH ecosystems answer to, for one project directory:
 * `fmt` and `lint`.
 *
 * They live together because three directories in this repo are a cargo crate
 * AND a TS package — `libs/contracts/tasks` and `libs/domains/todo` (ts-rs
 * bindings beside the crate that generates them), `libs/native/field-selector`
 * (an N-API addon with a vitest suite). One name there has to mean both
 * toolchains, so the target is COMPOSED from what the directory holds rather
 * than split into `lint-rust`/`lint-web`, which would push the "which one does
 * this project need" question back onto every caller.
 *
 * biome is the only JS/TS linter and formatter in this workspace (no eslint, no
 * prettier) and there is exactly one `biome.json` at the root, so a per-project
 * invocation is the same tool with a narrower path — no per-project config to
 * discover, which is also why `just lint-web` / `just fmt-web` stay whole-tree
 * single-process passes and are NOT built on these targets.
 *
 * Division of labour, same as `rust-targets.ts` describes for cargo: the `just`
 * recipes are the authoritative whole-repo gates (`cargo clippy --workspace`,
 * `biome ci .`, `cargo fmt --all`); these targets are the scope those recipes
 * cannot express — what a diff touched, with Nx Cloud caching the answer per
 * project.
 *
 * `plugin.ts` owns the nx registration; this file is imported into that single
 * plugin worker rather than registered as a plugin of its own.
 */

import { existsSync } from 'node:fs';
import { join } from 'node:path';

import type { CargoCrate } from './butler-config.ts';

/**
 * Every workspace package manifest. Rooted at `apps`/`libs` so the root
 * `package.json` — which is the workspace itself, not a project — stays out.
 */
export const PACKAGE_MANIFESTS = '{apps,libs}/**/package.json';

/**
 * `fmt` and `lint` for one directory, or undefined when it is neither a crate
 * nor a package. `excluded` marks a `[workspace] exclude`d crate: cargo cannot
 * resolve its package from the workspace root, so it is addressed by manifest
 * path where that works (rustfmt) and skipped where it does not (clippy, which
 * would build a wasm32-only crate for the host).
 */
export function polyglotTargets(
  workspaceRoot: string,
  dir: string,
  crate: CargoCrate | undefined,
  excluded: boolean,
): Record<string, unknown> | undefined {
  const isPackage = existsSync(join(workspaceRoot, dir, 'package.json'));
  if (!crate && !isPackage) return undefined;

  const fmt: string[] = [];
  const lint: string[] = [];
  const technologies: string[] = [];
  const lintInputs: unknown[] = ['default', '^default'];

  if (crate) {
    // `just fmt-rust`'s `cargo fmt --all` never reaches an excluded crate
    // either, so for `apps/todo/web-leptos` this target is the ONLY formatter
    // that does. `cargo sort` rides along because `just fmt-check-rust` gates
    // rustfmt AND dependency-table order — formatting without sorting hands you
    // a green target and a red gate. Same flags as the recipe (no `--grouped`):
    // a second spelling would rewrite what the workspace pass rewrites back.
    fmt.push(
      excluded
        ? `cargo fmt --manifest-path ${dir}/Cargo.toml --all`
        : `cargo fmt --package ${crate.name}`,
      `cargo sort ${dir}`,
    );
    technologies.push('rust');
    if (!excluded) {
      // Same flags as `just lint-rust`, one crate at a time.
      lint.push(
        `cargo clippy --package ${crate.name} --all-targets -- -D warnings`,
      );
      // What changes how every crate compiles; `nx.json` declares it.
      lintInputs.push('rustGlobals');
    }
  }

  if (isPackage) {
    // `--linter-enabled=false` on the format pass: `biome check --write` exits
    // 1 on an unfixable LINT diagnostic, and a formatter that fails on lint is
    // a gate wearing the wrong name. `biome ci` is the gate — read-only, and it
    // checks formatting and assists too, which is why `fmt-check` has no web
    // leaf.
    fmt.push(`bunx biome check --write --linter-enabled=false ${dir}`);
    lint.push(`bunx biome ci ${dir}`);
    technologies.push('typescript');
    lintInputs.push('{workspaceRoot}/biome.json', {
      externalDependencies: ['@biomejs/biome'],
    });
  }

  const targets: Record<string, unknown> = {
    // NEVER cached: rewriting files in place IS the output, nx fingerprints
    // those same files as inputs, and a cache hit would report success over a
    // tree someone has since reverted.
    fmt: {
      executor: 'nx:run-commands',
      cache: false,
      options: {
        commands: fmt,
        cwd: '{workspaceRoot}',
        // `cargo sort` reads the manifest `cargo fmt` may have just rewritten.
        parallel: false,
      },
      metadata: {
        description: `format ${dir} (${technologies.join(' + ')})`,
        technologies,
      },
    },
  };

  if (lint.length > 0) {
    // Cached with NO outputs, like the cargo gates in `rust-targets.ts`: a
    // cache entry means "these inputs passed clippy/biome", never "an artifact
    // was restored". `nx.json`'s `lint` targetDefault sets only `cache`, so
    // these inputs survive.
    targets.lint = {
      executor: 'nx:run-commands',
      cache: true,
      inputs: lintInputs,
      outputs: [],
      options: { commands: lint, cwd: '{workspaceRoot}', parallel: false },
      metadata: {
        description: `lint ${dir} (${technologies.join(' + ')})`,
        technologies,
      },
    };
  }

  return targets;
}
