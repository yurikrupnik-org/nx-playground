/**
 * Inferred `container` (image build) and `scan` (trivy) targets for one
 * deployable app.
 *
 * `@nx-tools/nx-container` ships the executor, not the wiring: each app used to
 * repeat ~40 lines of `project.json` naming its Dockerfile, its single build
 * arg, its tag, and the buildx/metadata block for CI. Seven apps carried that
 * block and it had drifted three ways — four had a registry layer cache and
 * three did not, `terran_*` scanned without `--cache-backend memory`, and three
 * genuinely deployable apps (`todo_api`, `todo_worker`, `todo-web`) had no
 * `container`/`scan` target at all, so CI never built or scanned their images
 * even though Tilt did.
 *
 * The inputs come from the same place butler's Tiltfile generator reads them:
 * the app's own `[image]` if it departs from the convention, else the repo
 * convention in the root `butler.toml` `[tilt.imageDefaults.<kind>]`. Those
 * files are the single source; this module and `butler container verify` are two
 * readers of it, and `just container-check` fails if they ever disagree.
 * The build STAGE is part of those shared facts, not a Tilt detail: the web
 * Dockerfile ends on `static-web-server`, which serves the SPA but drops the
 * `/api` reverse proxy every web Deployment configures through its proxy-envs
 * ConfigMap, so a target-less build silently shipped a different server than
 * dev validated. `[dockerfileTarget]` and `[imageDefaults.<kind>].target` sit at
 * the root of butler.toml for exactly that reason, and `butler container verify`
 * compares the emitted `target`.
 *
 * `plugin.ts` owns the nx registration; this file is imported into that single
 * plugin worker rather than registered as a plugin of its own.
 */

import {
  CONFIG_FILE,
  derivedImageName,
  type Kind,
  type RootConfig,
  readAppImage,
  readCargoCrate,
} from './butler-config.ts';

/**
 * Candidate apps: anything under `apps/` that could have a kind. The kind and
 * the deployability check below do the actual filtering — this glob only has to
 * be a superset, and it must cover the `[container] extra` apps too (they have
 * no k8s manifests to match on).
 */
export const APP_MARKER_FILES = [
  'Cargo.toml',
  'vite.config.ts',
  'astro.config.mjs',
] as const;

export const APP_MARKERS = `apps/**/{${APP_MARKER_FILES.join(',')}}`;

interface Image {
  /** Dockerfile, workspace-root-relative. */
  file: string;
  /** Build context, workspace-root-relative. */
  context: string;
  /** `KEY=VALUE` strings, as the executor takes them. */
  buildArgs: string[];
  /** Multi-stage build stage. Never left to the Dockerfile's last stage. */
  stage: string;
  /** Full reference including the tag, e.g. `$REGISTRY/zerg-api:latest`. */
  tag: string;
  /** The same reference without a tag, for metadata and the CVE scan. */
  repository: string;
  /** Bare image name, for the layer-cache ref and the SARIF filename. */
  name: string;
}

function resolveImage(
  workspaceRoot: string,
  root: RootConfig,
  appDir: string,
  kind: Kind,
): Image {
  const declared = readAppImage(workspaceRoot, appDir);
  let file: string;
  let context: string;
  let buildArgs: string[];
  let tag: string;
  let stage: string | undefined;

  if (declared) {
    file = declared.dockerfile;
    context = declared.context;
    buildArgs = Object.entries(declared.buildArgs).map(([k, v]) => `${k}=${v}`);
    tag = declared.tag;
    stage = declared.target;
  } else {
    const convention = root.imageDefaults[kind];
    if (!convention) {
      throw new Error(
        `${appDir}: no image inputs and no repo convention for a ${kind} — add ` +
          `[imageDefaults.${kind}] to the root ${CONFIG_FILE}, or an [image] ` +
          `section to ${appDir}/${CONFIG_FILE}`,
      );
    }
    file = convention.dockerfile;
    context = '.';
    buildArgs = [];
    if (convention.buildArg) {
      // The one per-app value the Dockerfile needs.
      let value: string;
      if (kind === 'service') {
        // Which crate to compile.
        const crate = readCargoCrate(workspaceRoot, appDir);
        if (!crate)
          throw new Error(`${appDir}: Cargo.toml has no [package] name`);
        value = crate.name;
      } else if (kind === 'web') {
        // Where the local build dropped the static output.
        value = `${appDir}/dist`;
      } else {
        // Which app to install and build inside the image: an SSR app's N-API
        // addon has to be compiled for the image's own platform, so there is
        // no host-built output to copy.
        value = appDir;
      }
      buildArgs = [`${convention.buildArg}=${value}`];
    }
    tag = `$REGISTRY/${derivedImageName(appDir)}:latest`;
    stage = convention.target;
  }

  // A stage the source did not pin comes from the repo-wide map. Leaving it to
  // the Dockerfile's last stage is what silently shipped `static-web-server`
  // (no `/api` proxy) where every web Deployment expects `nginx`.
  const resolved = stage ?? root.dockerfileTarget[file];
  if (!resolved) {
    throw new Error(
      `${appDir}: no build stage for ${file} — pin one on the image or add a ` +
        `[dockerfileTarget] entry to the root ${CONFIG_FILE}`,
    );
  }

  // A port in a registry host (`localhost:5000/x`) is not a tag.
  const lastColon = tag.lastIndexOf(':');
  const repository =
    lastColon > tag.lastIndexOf('/') ? tag.slice(0, lastColon) : tag;
  const name = repository.slice(repository.lastIndexOf('/') + 1);
  return {
    file,
    context,
    buildArgs,
    stage: resolved,
    tag,
    repository,
    name,
  };
}

/** Both targets for an app, with its image resolved from the config. */
export function containerTargets(
  workspaceRoot: string,
  root: RootConfig,
  appDir: string,
  kind: Kind,
): Record<string, unknown> {
  const image = resolveImage(workspaceRoot, root, appDir, kind);
  return containerAndScan(image, kind);
}

function containerAndScan(image: Image, kind: Kind): Record<string, unknown> {
  return {
    container: {
      executor: '@nx-tools/nx-container:build',
      // A web image is the static output of the local build; a service is
      // compiled inside the image, so nothing has to happen first.
      ...(kind === 'web' ? { dependsOn: ['build'] } : {}),
      options: {
        file: image.file,
        context: image.context,
        'build-args': image.buildArgs,
        target: image.stage,
        tags: [image.tag],
        push: false,
      },
      configurations: {
        ci: {
          push: true,
          'cache-from': [`type=registry,ref=$BUILDCACHE/${image.name}`],
          'cache-to': [
            `type=registry,ref=$BUILDCACHE/${image.name},mode=max,image-manifest=true,oci-mediatypes=true`,
          ],
          metadata: {
            images: [image.repository],
            tags: [
              'type=sha',
              'type=ref,event=branch',
              'type=ref,event=pr',
              'type=raw,value=$APP_VERSION,enable=$ENABLE_VERSION',
            ],
          },
        },
      },
      metadata: {
        description: `Build ${image.repository} from ${image.file}`,
        technologies: ['docker'],
      },
    },
    scan: {
      executor: 'nx:run-commands',
      dependsOn: ['container'],
      options: {
        command: `trivy image --cache-backend memory ${image.tag} --severity CRITICAL,HIGH --exit-code 0`,
        cwd: '{workspaceRoot}',
      },
      configurations: {
        // CI scans the immutable sha tag the push produced and reports through
        // GitHub code scanning, so the SARIF filename must be app-unique.
        ci: {
          command:
            `trivy image --cache-backend memory ${image.repository}:sha-$(echo $SHORT_SHA | cut -c1-7) ` +
            `--severity CRITICAL,HIGH --format sarif --output trivy-${image.name}.sarif`,
        },
      },
      metadata: {
        description: `Scan ${image.repository} for CRITICAL/HIGH CVEs`,
        technologies: ['docker'],
      },
    },
  };
}
