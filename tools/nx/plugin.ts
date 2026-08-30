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
 *   rust-targets.ts       `build` / `run`            — every cargo crate
 *   container-targets.ts  `container` / `scan`       — every deployable app
 *
 * Zero dependencies on purpose: `createNodesV2` is a plain export, so nothing
 * here needs `@nx/devkit` or `@nx/plugin` installed.
 */

import { dirname } from 'node:path';

import {
  appKind,
  type CreateNodesV2,
  derivedImageName,
  hasWorkload,
  isProject,
  type RootConfig,
  readCargoCrate,
  readRootConfig,
} from './butler-config.ts';
import {
  APP_MARKER_FILES,
  APP_MARKERS,
  containerTargets,
} from './container-targets.ts';
import { k8sTargets } from './k8s-targets.ts';
import { CARGO_MANIFESTS, rustTargets } from './rust-targets.ts';
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
const MARKERS = `{${CARGO_MANIFESTS},${APP_MARKERS},${APP_CONFIGS}}`;

type Entry = [
  string,
  { projects: Record<string, { targets: Record<string, unknown> }> },
];

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
    const results: Entry[] = [];
    const containerApps = new Set<string>();
    const workloadApps = new Set<string>();

    for (const file of files) {
      const dir = dirname(file);

      if (file.endsWith('/Cargo.toml')) {
        // A manifest with no `[package]` is a nested workspace, not a crate.
        const crate = readCargoCrate(workspaceRoot, dir);
        if (crate) {
          results.push([
            file,
            { projects: { [dir]: { targets: rustTargets(crate) } } },
          ]);
        }
      }

      if (APP_MARKER_FILES.some((marker) => file.endsWith(`/${marker}`))) {
        if (dir.startsWith('apps/') && !containerApps.has(dir)) {
          const entry = containerEntry(workspaceRoot, root, dir);
          if (entry) {
            containerApps.add(dir);
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
          results.push([
            file,
            {
              projects: {
                [dir]: {
                  targets: {
                    ...tiltTargets(dir),
                    ...k8sTargets(dir, derivedImageName(dir), root.k8sOutDir),
                  },
                },
              },
            },
          ]);
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
