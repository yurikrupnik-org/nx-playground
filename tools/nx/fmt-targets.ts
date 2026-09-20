/**
 * Inferred `fmt` target: one name per project, running every formatter that
 * project's sources actually need.
 *
 * It does NOT replace `just fmt-rust` / `just fmt-web`, and must not be wired
 * into `just fmt`. Those stay the whole-repo pass for the same reason
 * `rust-targets.ts` keeps `just lint-rust` cargo-direct: one `cargo fmt --all`
 * and one biome process over the tree beat N per-project tasks, and there is no
 * per-project biome config to honour. This target is the scope those recipes
 * cannot express — format exactly what a diff touched, `nx affected -t fmt`.
 *
 * COMPOSED, not one target per ecosystem: `libs/contracts/tasks`,
 * `libs/domains/todo` and `libs/native/field-selector` are a cargo crate AND a
 * TS package in one directory (ts-rs bindings, an N-API addon), so `fmt` there
 * has to mean both. A `fmt-rust`/`fmt-web` pair would make the caller know
 * which one a project needs, which is the fact this module already derives.
 *
 * `cargo sort` rides along because `just fmt-check-rust` gates rustfmt AND
 * dependency-table order: a `fmt` that formats but leaves the manifest unsorted
 * hands you a green target and a red gate. Same flags as the recipe (plain
 * `cargo sort`, no `--grouped`) — a second spelling would rewrite manifests the
 * workspace pass then rewrites back.
 *
 * NEVER cached: rewriting files in place IS the output, nx fingerprints the
 * same files as inputs, and a cache hit would report success over a source tree
 * someone has since reverted.
 *
 * `plugin.ts` owns the nx registration; this file is imported into that single
 * plugin worker rather than registered as a plugin of its own.
 */

import { existsSync } from 'node:fs';
import { join } from 'node:path';

import type { CargoCrate } from './butler-config.ts';

/**
 * Every workspace package manifest. Rooted at `apps`/`libs` so the root
 * `package.json` — which is the workspace itself, not a project — stays out;
 * the monodocs fixture tree is excluded in `.nxignore`, like its crates.
 */
export const PACKAGE_MANIFESTS = '{apps,libs}/**/package.json';

/**
 * `fmt` for one directory, or undefined when nothing in it is formattable by
 * this repo's formatters. `excluded` marks a `[workspace] exclude`d crate.
 */
export function fmtTarget(
  workspaceRoot: string,
  dir: string,
  crate: CargoCrate | undefined,
  excluded: boolean,
): Record<string, unknown> | undefined {
  const commands: string[] = [];
  const technologies: string[] = [];
  if (crate) {
    // An excluded crate is its own workspace, so `--package` cannot resolve it
    // from the root — and `just fmt-rust`'s `cargo fmt --all` misses it for the
    // same reason, which makes this target the ONLY formatter that reaches
    // `apps/todo/web-leptos`. `cargo sort` takes a directory either way.
    commands.push(
      excluded
        ? `cargo fmt --manifest-path ${dir}/Cargo.toml --all`
        : `cargo fmt --package ${crate.name}`,
      `cargo sort ${dir}`,
    );
    technologies.push('rust');
  }
  if (existsSync(join(workspaceRoot, dir, 'package.json'))) {
    // Mirrors `just fmt-web`: formatter and assists only. `--linter-enabled=false`
    // because `biome check --write` exits 1 on an unfixable LINT diagnostic, and
    // a formatting pass that fails on lint is a gate wearing the wrong name —
    // `just lint-web` (`biome ci .`) owns that.
    commands.push(`bunx biome check --write --linter-enabled=false ${dir}`);
    technologies.push('typescript');
  }
  if (commands.length === 0) return undefined;
  return {
    fmt: {
      executor: 'nx:run-commands',
      cache: false,
      options: {
        commands,
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
}
