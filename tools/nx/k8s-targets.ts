/**
 * Inferred manifest-generation targets (`k8s-gen` / `k8s-check`) for one app.
 *
 * Same division as the Tiltfile targets: nx owns the LIST — an app qualifies by
 * declaring a `[workload]` in its own `butler.toml` — and butler owns the LOGIC.
 * It merges `[workloadDefaults.<kind>]` under the app's `[workload]` under any
 * `[env.<env>.workload]` overlay, injects what an app must never hand-type (the
 * image reference, name, namespace, partOf), writes `<app>/k8s/values.yaml`, and
 * renders that through the KCL package pinned in the root `[k8s]` section.
 *
 * Why the image is injected rather than declared: before this, the same image
 * fact was spelled three ways — `yurikrupnik/todo-api:dev` in a manifest,
 * `yurikrupnik/zerg-api:main` in another, `$REGISTRY/<name>:latest` in the nx
 * container target. One home, one spelling, or they drift.
 *
 * `plugin.ts` owns the nx registration; this file is imported into that single
 * plugin worker rather than registered as a plugin of its own.
 */

/** Everything that can change an app's values file or its rendered manifests. */
function inputs(appDir: string): unknown[] {
  return [
    { externalDependencies: [] },
    `{workspaceRoot}/${appDir}/butler.toml`,
    // The kind decides which `[workloadDefaults.<kind>]` applies.
    `{workspaceRoot}/${appDir}/Cargo.toml`,
    `{workspaceRoot}/${appDir}/vite.config.ts`,
    `{workspaceRoot}/${appDir}/astro.config.mjs`,
    // Repo-wide config: registry, env, image conventions, workload defaults, and
    // the pinned KCL package reference.
    '{workspaceRoot}/butler.toml',
    // The generator itself.
    '{workspaceRoot}/apps/butler/cli/src/**/*.rs',
  ];
}

export function k8sTargets(
  appDir: string,
  imageName: string,
  outDir: string,
): Record<string, unknown> {
  const gen = `cargo run --quiet -p butler -- k8s gen --app ${appDir}`;
  const rendered = `{workspaceRoot}/${outDir}/${imageName}.yaml`;
  return {
    'k8s-gen': {
      command: gen,
      options: { cwd: '{workspaceRoot}' },
      cache: true,
      inputs: inputs(appDir),
      outputs: [`{projectRoot}/k8s/values.yaml`, rendered],
      metadata: {
        description: `Generate ${appDir}/k8s/values.yaml and render it to ${outDir}/${imageName}.yaml`,
        technologies: ['kubernetes'],
      },
    },
    'k8s-check': {
      command: `${gen} --check`,
      options: { cwd: '{workspaceRoot}' },
      cache: true,
      inputs: [
        ...inputs(appDir),
        `{workspaceRoot}/${appDir}/k8s/values.yaml`,
        rendered,
      ],
      metadata: {
        description: `Fail if ${imageName}'s values or rendered manifests have drifted from butler.toml`,
        technologies: ['kubernetes'],
      },
    },
  };
}
