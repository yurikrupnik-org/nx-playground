/**
 * Inferred Tiltfile generation targets (`tilt-gen` / `tilt-check`) for one app.
 *
 * nx owns the *list* — the marker glob below decides which projects are Tilt
 * apps, and `nx run-many -t tilt-gen` is therefore the authoritative app set. butler
 * owns the *logic*: the tight `docker_build(only=...)` needs the transitive,
 * dev-dependency-excluding cargo closure, which nx's graph cannot express
 * (`@monodon/rust` flattens `[dependencies]` and `[dev-dependencies]` into one
 * edge type, so deriving `only=` from it would drag `libs/testing/test-utils`
 * into every service image). Reimplementing that here would mean a second graph
 * that can disagree with the first; instead each target shells out to
 * `butler tilt gen --app <dir>`.
 *
 * `plugin.ts` owns the nx registration; this file is imported into that single
 * plugin worker rather than registered as a plugin of its own, and shares the
 * app predicates with its siblings through `butler-config.ts`.
 *
 * An app qualifies exactly as it does for butler: it declares a `[workload]` in
 * its own `butler.toml` and has a recognizable kind (`Cargo.toml` -> service,
 * `vite.config.ts` -> web, `astro.config.mjs` -> node). Directories that ship
 * manifests but no workload, such as `apps/zerg/shared`, are skipped.
 */

/** Everything that can change a generated Tiltfile. */
function inputs(appDir: string): unknown[] {
  return [
    { externalDependencies: [] },
    `{workspaceRoot}/${appDir}/butler.toml`,
    `{workspaceRoot}/${appDir}/project.json`,
    `{workspaceRoot}/${appDir}/Cargo.toml`,
    `{workspaceRoot}/${appDir}/vite.config.ts`,
    `{workspaceRoot}/${appDir}/index.html`,
    `{workspaceRoot}/${appDir}/astro.config.mjs`,
    // Repo-wide config and the graph inputs the `only=` list is derived from.
    '{workspaceRoot}/butler.toml',
    '{workspaceRoot}/Cargo.toml',
    '{workspaceRoot}/Cargo.lock',
    '{workspaceRoot}/apps/*/*/Cargo.toml',
    '{workspaceRoot}/libs/**/Cargo.toml',
    // The generator itself.
    '{workspaceRoot}/apps/butler/cli/src/**/*.rs',
  ];
}

export function tiltTargets(appDir: string): Record<string, unknown> {
  const gen = `cargo run --quiet -p butler -- tilt gen --app ${appDir}`;
  return {
    'tilt-gen': {
      command: gen,
      options: { cwd: '{workspaceRoot}' },
      cache: true,
      inputs: inputs(appDir),
      outputs: [`{projectRoot}/Tiltfile`],
      metadata: {
        description: `Generate ${appDir}/Tiltfile from butler.toml + the project graph`,
        technologies: ['tilt'],
      },
    },
    'tilt-check': {
      command: `${gen} --check`,
      options: { cwd: '{workspaceRoot}' },
      cache: true,
      inputs: [...inputs(appDir), `{workspaceRoot}/${appDir}/Tiltfile`],
      metadata: {
        description: `Fail if ${appDir}/Tiltfile has drifted from the generator`,
        technologies: ['tilt'],
      },
    },
  };
}

// The root Tiltfile is a workspace-level artifact (infra port-forwards, shared
// resources, one include() per app), so there is no project to hang it off: this
// workspace has no root project, and adding one would change `nx affected`
// semantics for every file. `just tilt-gen` derives its app list from the nx
// graph instead, so nx stays authoritative there too:
//
//   nodes with a `tilt-gen` target -> their roots -> butler tilt gen --root --apps <roots>
