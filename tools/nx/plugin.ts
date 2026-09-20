/**
 * The repo's only local nx plugin. One registration, three inference modules.
 *
 * Why one: nx forks an isolated worker process per REGISTERED plugin, and each
 * one costs ~70 MB RSS plus its own startup while the project graph is computed.
 * Three separate plugin entries measured ~70 ms and ~140 MB more per graph build
 * than this single entry does, for exactly the same targets — so the split lives
 * in the module boundaries below, not in `nx.json`.
 *
 * What each module contributes, all merged onto EXISTING graph nodes (this
 * plugin creates none of its own):
 *
 *   tilt-targets.ts       `tilt-gen` / `tilt-check`  — apps that ship k8s manifests
 *   rust-targets.ts       `build` / `lint` / `test` / `run` / `install` — cargo crates
 *   container-targets.ts  `container` / `scan`       — every deployable app
 *   fmt-targets.ts        `fmt`                      — crates and TS packages
 *
 * Zero dependencies on purpose: `createNodesV2` is a plain export, so nothing
 * here needs `@nx/devkit` or `@nx/plugin` installed.
 */

import { dirname } from 'node:path';

import {
  appKind,
  type CargoCrate,
  type CreateNodesV2,
  derivedImageName,
  hasWorkload,
  isProject,
  type ProjectContribution,
  type RootConfig,
  readCargoCrate,
  readCargoExclusions,
  readRootConfig,
} from './butler-config.ts';
import {
  APP_MARKER_FILES,
  APP_MARKERS,
  containerTargets,
} from './container-targets.ts';
import { fmtTarget, PACKAGE_MANIFESTS } from './fmt-targets.ts';
import { k8sTargets } from './k8s-targets.ts';
import {
  CARGO_MANIFESTS,
  hasPackageScriptGates,
  RUST_TAG,
  rustTargets,
} from './rust-targets.ts';
import { scopeTag } from './scope-tags.ts';
import { tiltTargets } from './tilt-targets.ts';

/**
 * Every app config file: an app declares its workload there, so that file is
 * what makes it a Tilt app and a manifest-generating app. Deliberately NOT the
 * k8s manifests it used to be keyed on — those are now GENERATED from this very
 * file, and keying inference on your own output is circular.
 */
const APP_CONFIGS = 'apps/**/butler.toml';

/**
 * One glob per inference module, braced into the single pattern nx matches. A
 * file can feed more than one module — `apps/zerg/api/Cargo.toml` is both a
 * crate and a deployable app — so the callback dispatches on what the path is,
 * not on which alternative matched.
 */
const MARKERS = `{${CARGO_MANIFESTS},${APP_MARKERS},${APP_CONFIGS},${PACKAGE_MANIFESTS}}`;

type Entry = [string, { projects: Record<string, ProjectContribution> }];

/**
 * `container`/`scan` for an app that has a kind and is deployed: declaring a
 * `[workload]` is what deploys an app, so it is what makes an image worth
 * building, and an app deployed from outside this repo opts in through
 * `[container] extra`.
 */
function containerEntry(
  workspaceRoot: string,
  root: RootConfig,
  appDir: string,
): Entry[1] | undefined {
  const kind = appKind(workspaceRoot, appDir);
  if (!kind || !isProject(workspaceRoot, appDir)) return undefined;
  if (
    !hasWorkload(workspaceRoot, appDir) &&
    !root.containerExtra.includes(appDir)
  ) {
    return undefined;
  }
  return {
    projects: {
      [appDir]: {
        targets: containerTargets(workspaceRoot, root, appDir, kind),
      },
    },
  };
}

export const createNodesV2: CreateNodesV2 = [
  MARKERS,
  async (files, _options, context) => {
    const workspaceRoot = context.workspaceRoot;
    const root = readRootConfig(workspaceRoot);
    // Read once per graph build, like the root config above.
    const excludedCrates = readCargoExclusions(workspaceRoot);
    const results: Entry[] = [];
    const containerApps = new Set<string>();
    const workloadApps = new Set<string>();

    // `scope:` goes on the FIRST contribution for a dir only — nx CONCATs tags
    // from every contribution, so a second copy would show up duplicated.
    const tagged = new Set<string>();
    const scopeFor = (dir: string): string[] => {
      if (tagged.has(dir)) return [];
      tagged.add(dir);
      return [scopeTag(dir)];
    };

    // One parse per crate manifest: a directory that is both a crate and a TS
    // package (ts-rs bindings, the N-API addons) reaches the cargo branch and
    // the `fmt` branch below, and either marker file can arrive first.
    const crates = new Map<string, CargoCrate | undefined>();
    const crateAt = (dir: string): CargoCrate | undefined => {
      if (!crates.has(dir)) crates.set(dir, readCargoCrate(workspaceRoot, dir));
      return crates.get(dir);
    };
    const formatted = new Set<string>();

    for (const file of files) {
      const dir = dirname(file);

      if (file.endsWith('/Cargo.toml')) {
        // A manifest with no `[package]` is a nested workspace, not a crate.
        const crate = crateAt(dir);
        if (crate) {
          // A `[workspace] exclude`d directory is a crate but not a MEMBER, so
          // `cargo … --package <name>` from the workspace root cannot resolve
          // it: no cargo targets, and above all no `rust` tag, which would
          // hand it to `just check-rust-affected`. It still belongs to a
          // vertical, so it keeps its scope tag. (`apps/todo/web-leptos`: a
          // wasm32 trunk app whose `build` comes from its own project.json.)
          const excluded = excludedCrates.has(dir);
          // The tag is what lets a gate address the Rust half of the graph
          // (`-p tag:rust`); nothing else in the graph marks a node as a cargo
          // crate. It is withheld from a crate whose `lint`/`test` come from a
          // package.json script — the N-API addons — because that lint is
          // mutating and those are `just test-napi`'s job.
          const tags = scopeFor(dir);
          if (!excluded && !hasPackageScriptGates(workspaceRoot, dir))
            tags.push(RUST_TAG);
          const contribution: ProjectContribution = {
            targets: excluded ? {} : rustTargets(crate, dir),
          };
          if (tags.length > 0) contribution.tags = tags;
          results.push([file, { projects: { [dir]: contribution } }]);
        }
      }

      if (APP_MARKER_FILES.some((marker) => file.endsWith(`/${marker}`))) {
        if (dir.startsWith('apps/') && !containerApps.has(dir)) {
          const entry = containerEntry(workspaceRoot, root, dir);
          if (entry) {
            containerApps.add(dir);
            const tags = scopeFor(dir);
            if (tags.length > 0) entry.projects[dir].tags = tags;
            results.push([file, entry]);
          }
        }
      }

      if (file.endsWith('/butler.toml') && dir.startsWith('apps/')) {
        if (
          !workloadApps.has(dir) &&
          appKind(workspaceRoot, dir) &&
          isProject(workspaceRoot, dir) &&
          hasWorkload(workspaceRoot, dir)
        ) {
          workloadApps.add(dir);
          const contribution: ProjectContribution = {
            targets: {
              ...tiltTargets(dir),
              ...k8sTargets(dir, derivedImageName(dir), root.k8sOutDir),
            },
          };
          const tags = scopeFor(dir);
          if (tags.length > 0) contribution.tags = tags;
          results.push([file, { projects: { [dir]: contribution } }]);
        }
      }

      // `fmt` is the one target both ecosystems answer to, so it is keyed on
      // either manifest and emitted once per directory — whichever of the two
      // nx hands us first. No `scope:` tag from here: a TS-only project such as
      // `apps/todo/e2e` declares its own in project.json, and a second copy
      // would concat onto it.
      if (
        !formatted.has(dir) &&
        (file.endsWith('/Cargo.toml') || file.endsWith('/package.json'))
      ) {
        const target = fmtTarget(
          workspaceRoot,
          dir,
          crateAt(dir),
          excludedCrates.has(dir),
        );
        if (target) {
          formatted.add(dir);
          results.push([file, { projects: { [dir]: { targets: target } } }]);
        }
      }
    }

    const missing = root.containerExtra.filter(
      (dir) => !containerApps.has(dir),
    );
    if (missing.length > 0) {
      throw new Error(
        `butler.toml [container] extra lists ${missing.join(', ')}, which is not an ` +
          'app with a recognizable kind (a Cargo.toml or a vite.config.ts) under apps/',
      );
    }
    return results;
  },
];

// The root Tiltfile is a workspace-level artifact (infra port-forwards, shared
// resources, one include() per app), so there is no project to hang it off: this
// workspace has no root project, and adding one would change `nx affected`
// semantics for every file. `just tilt-gen` derives its app list from the nx
// graph instead, so nx stays authoritative there too:
//
//   nodes with a `tilt-gen` target -> their roots -> butler tilt gen --root --apps <roots>
