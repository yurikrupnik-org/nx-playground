//! The runner Tasks every generated Pipeline calls: one per tool, each taking
//! the target as a param, so a Pipeline carries only *which* target it runs.
//!
//! Params reach shell scripts through `env`, never spliced into the script
//! text: Tekton substitutes `$(params.x)` textually, so a param in a script is
//! shell injection waiting for a revision name with a `;` in it. Params in
//! `command`/`args` are argv entries and need no such care.

use serde_yaml_ng::{Mapping, Value as Y};

use crate::submodule::manifests::{map, s};

/// Where the task/just runners drop the downloaded binary.
const BIN_DIR: &str = "/butler/bin";
const BIN_VOLUME: &str = "butler-bin";

pub(super) const CLONE: &str = "butler-git-clone";
pub(super) const NX: &str = "butler-nx";
pub(super) const TASK: &str = "butler-task";
pub(super) const JUST: &str = "butler-just";
pub(super) const NU: &str = "butler-nu";
pub(super) const BUILDKIT: &str = "butler-buildkit";
pub(super) const APPLY: &str = "butler-kubectl-apply";

/// The workspace every runner works in: the clone.
pub(super) const SOURCE: &str = "source";
/// Optional registry credentials (a `config.json`) for pushing images.
pub(super) const DOCKERCONFIG: &str = "dockerconfig";

pub(super) fn git_clone(image: &str) -> Y {
    task(
        CLONE,
        "Shallow-clone one revision of the repository into the source workspace.",
        vec![
            param("url", "Git URL to clone."),
            param("revision", "Branch, tag or commit to check out."),
        ],
        vec![workspace(SOURCE, "Receives the checkout.")],
        Vec::new(),
        Vec::new(),
        vec![script_step(
            "clone",
            image,
            &[("URL", "$(params.url)"), ("REVISION", "$(params.revision)")],
            "#!/bin/sh\n\
             set -eu\n\
             cd \"$(workspaces.source.path)\"\n\
             # A reused volume still holds the previous checkout.\n\
             find . -mindepth 1 -maxdepth 1 -exec rm -rf {} +\n\
             git init -q .\n\
             git remote add origin \"$URL\"\n\
             git fetch -q --depth 1 origin \"$REVISION\"\n\
             git checkout -q FETCH_HEAD\n\
             git log -1 --format='%H %s'\n",
        )],
    )
}

pub(super) fn nx(image: &str) -> Y {
    task(
        NX,
        "Install the JS dependencies, then `bun nx run <project>:<target>`.",
        vec![
            param("project", "nx project name."),
            param("target", "nx target name."),
            args_param(),
        ],
        vec![workspace(SOURCE, "The checkout.")],
        Vec::new(),
        Vec::new(),
        vec![
            in_source(script_step(
                "install",
                image,
                &[],
                "#!/bin/sh\nset -eu\nbun install --frozen-lockfile\n",
            )),
            in_source(argv_step(
                "run",
                image,
                &[("CI", "true"), ("NX_DAEMON", "false")],
                &["bun", "nx", "run", "$(params.project):$(params.target)"],
            )),
        ],
    )
}

/// A downloaded runner binary: its release is pinned in butler.toml and its
/// archive checked against the release's own checksum list.
pub(super) struct Release<'a> {
    pub fetch_image: &'a str,
    pub version: &'a str,
}

pub(super) fn go_task(image: &str, release: &Release<'_>) -> Y {
    downloaded(
        TASK,
        "Download go-task, then `task <task>` in the source workspace.",
        (
            "task",
            "Taskfile task name, namespaced as `task --list-all` prints it.",
        ),
        "task",
        &["$(params.task)"],
        image,
        release,
        "case \"$(uname -m)\" in\n  \
           x86_64) arch=amd64 ;;\n  \
           aarch64 | arm64) arch=arm64 ;;\n  \
           *) echo \"no go-task build for $(uname -m)\" >&2; exit 1 ;;\n\
         esac\n\
         archive=\"task_linux_${arch}.tar.gz\"\n\
         member=task\n\
         sums=task_checksums.txt\n\
         base=\"https://github.com/go-task/task/releases/download/v${VERSION}\"\n",
    )
}

pub(super) fn just(image: &str, release: &Release<'_>) -> Y {
    downloaded(
        JUST,
        "Download just, then `just <recipe>` in the source workspace.",
        ("recipe", "Recipe path, `module::recipe` inside a module."),
        "just",
        &["$(params.recipe)"],
        image,
        release,
        "case \"$(uname -m)\" in\n  \
           x86_64) arch=x86_64 ;;\n  \
           aarch64 | arm64) arch=aarch64 ;;\n  \
           *) echo \"no just build for $(uname -m)\" >&2; exit 1 ;;\n\
         esac\n\
         archive=\"just-${VERSION}-${arch}-unknown-linux-musl.tar.gz\"\n\
         member=just\n\
         sums=SHA256SUMS\n\
         base=\"https://github.com/casey/just/releases/download/${VERSION}\"\n",
    )
}

/// nu is downloaded too: the nushell image carries nu and nothing else, while
/// scripts shell out to git, kubectl and friends.
pub(super) fn nu(image: &str, release: &Release<'_>) -> Y {
    downloaded(
        NU,
        "Download nu, then `nu --no-config-file <script>` in the source workspace.",
        ("script", "Workspace-relative .nu path."),
        "nu",
        // --no-config-file: a user's config.nu must not change what a script
        // does (scripts/tasks/nu.yml), and a pod has none anyway.
        &["--no-config-file", "$(params.script)"],
        image,
        release,
        "case \"$(uname -m)\" in\n  \
           x86_64) arch=x86_64 ;;\n  \
           aarch64 | arm64) arch=aarch64 ;;\n  \
           *) echo \"no nu build for $(uname -m)\" >&2; exit 1 ;;\n\
         esac\n\
         dir=\"nu-${VERSION}-${arch}-unknown-linux-musl\"\n\
         archive=\"${dir}.tar.gz\"\n\
         member=\"${dir}/nu\"\n\
         sums=SHA256SUMS\n\
         base=\"https://github.com/nushell/nushell/releases/download/${VERSION}\"\n",
    )
}

/// A runner whose binary no image worth running targets in ships: a `fetch`
/// step downloads the pinned release into a shared volume, and the `run` step
/// executes it in the toolchain image the targets actually need. `locate`
/// sets `archive`, `member` (the binary's path inside it), `sums` and `base`
/// for the running architecture.
#[allow(clippy::too_many_arguments)]
fn downloaded(
    name: &str,
    description: &str,
    (target, target_description): (&str, &str),
    binary: &str,
    argv: &[&str],
    image: &str,
    release: &Release<'_>,
    locate: &str,
) -> Y {
    let fetch = format!(
        "#!/bin/sh\n\
         set -eu\n\
         {locate}\
         cd {BIN_DIR}\n\
         wget -q \"$base/$archive\" \"$base/$sums\"\n\
         grep \" $archive\\$\" \"$sums\" | sha256sum -c -\n\
         tar -xzf \"$archive\" \"$member\"\n\
         if [ \"$member\" != {binary} ]; then mv \"$member\" {binary}; rm -rf \"${{member%%/*}}\"; fi\n\
         rm \"$archive\" \"$sums\"\n"
    );
    let program = format!("{BIN_DIR}/{binary}");
    let command: Vec<&str> = std::iter::once(program.as_str())
        .chain(argv.iter().copied())
        .collect();
    task(
        name,
        description,
        vec![param(target, target_description), args_param()],
        vec![workspace(SOURCE, "The checkout.")],
        Vec::new(),
        vec![map([
            ("name", s(BIN_VOLUME)),
            ("emptyDir", Y::Mapping(Mapping::new())),
        ])],
        vec![
            with_bin(script_step(
                &format!("fetch-{binary}"),
                release.fetch_image,
                &[("VERSION", release.version)],
                &fetch,
            )),
            with_bin(in_source(argv_step("run", image, &[], &command))),
        ],
    )
}

pub(super) fn buildkit(image: &str) -> Y {
    let mut step = script_step(
        "build",
        image,
        &[
            ("IMAGE", "$(params.image)"),
            ("DOCKERFILE", "$(params.dockerfile)"),
            ("CONTEXT", "$(params.context)"),
            ("TARGET", "$(params.target)"),
            ("DOCKERCONFIG_BOUND", "$(workspaces.dockerconfig.bound)"),
            ("DOCKERCONFIG_PATH", "$(workspaces.dockerconfig.path)"),
            ("DIGEST_PATH", "$(results.digest.path)"),
            // Rootless buildkitd cannot create its own PID namespace in a pod.
            ("BUILDKITD_FLAGS", "--oci-worker-no-process-sandbox"),
        ],
        "#!/bin/sh\n\
         set -eu\n\
         if [ \"$DOCKERCONFIG_BOUND\" = true ]; then export DOCKER_CONFIG=\"$DOCKERCONFIG_PATH\"; fi\n\
         # \"$@\" is the build-args param: one KEY=VALUE per entry.\n\
         n=$#\n\
         while [ \"$n\" -gt 0 ]; do set -- \"$@\" --opt \"build-arg:$1\"; shift; n=$((n - 1)); done\n\
         buildctl-daemonless.sh build \\\n  \
           --frontend dockerfile.v0 \\\n  \
           --local context=\"$CONTEXT\" \\\n  \
           --local dockerfile=\"$(dirname \"$DOCKERFILE\")\" \\\n  \
           --opt filename=\"$(basename \"$DOCKERFILE\")\" \\\n  \
           --opt target=\"$TARGET\" \\\n  \
           \"$@\" \\\n  \
           --output type=image,name=\"$IMAGE\",push=true \\\n  \
           --metadata-file /tmp/metadata.json\n\
         sed -n 's/.*\"containerimage.digest\": *\"\\([^\"]*\\)\".*/\\1/p' /tmp/metadata.json | tr -d '\\n' > \"$DIGEST_PATH\"\n\
         test -s \"$DIGEST_PATH\" || { echo 'buildctl reported no image digest' >&2; exit 1; }\n",
    );
    set(&mut step, "workingDir", s("$(workspaces.source.path)"));
    set(
        &mut step,
        "args",
        Y::Sequence(vec![s("$(params.build-args[*])")]),
    );
    set(
        &mut step,
        "securityContext",
        map([
            ("runAsUser", Y::Number(1000.into())),
            ("runAsGroup", Y::Number(1000.into())),
            ("seccompProfile", map([("type", s("Unconfined"))])),
            ("appArmorProfile", map([("type", s("Unconfined"))])),
        ]),
    );
    task(
        BUILDKIT,
        "Build one Dockerfile stage with rootless buildkit and push it.",
        vec![
            param("image", "Reference to push, with its tag."),
            param("dockerfile", "Workspace-relative Dockerfile."),
            param("context", "Workspace-relative build context."),
            param("target", "Dockerfile stage to build."),
            map([
                ("name", s("build-args")),
                ("type", s("array")),
                ("description", s("KEY=VALUE build arguments.")),
                ("default", Y::Sequence(Vec::new())),
            ]),
        ],
        vec![
            workspace(SOURCE, "The checkout."),
            map([
                ("name", s(DOCKERCONFIG)),
                (
                    "description",
                    s("Registry credentials: a docker config.json."),
                ),
                ("optional", Y::Bool(true)),
            ]),
        ],
        vec![map([
            ("name", s("digest")),
            ("type", s("string")),
            ("description", s("Digest of the pushed image.")),
        ])],
        Vec::new(),
        vec![step],
    )
}

pub(super) fn kubectl_apply(image: &str) -> Y {
    task(
        APPLY,
        "Apply a rendered manifest with its image pinned to the digest just built.",
        vec![
            param("manifest", "Workspace-relative rendered manifest."),
            param(
                "image",
                "The image reference the manifest names, with its tag.",
            ),
            param("digest", "Digest to pin that reference to."),
        ],
        vec![workspace(SOURCE, "The checkout.")],
        Vec::new(),
        Vec::new(),
        vec![in_source(script_step(
            "apply",
            image,
            &[
                ("MANIFEST", "$(params.manifest)"),
                ("IMAGE", "$(params.image)"),
                ("DIGEST", "$(params.digest)"),
            ],
            // Tilt's image injection, by digest: the committed manifest names
            // the env tag, and a tag that did not move rolls nothing out.
            "#!/bin/sh\n\
             set -eu\n\
             pinned=\"${IMAGE%:*}@${DIGEST}\"\n\
             sed \"s|image: ${IMAGE}\\$|image: ${pinned}|\" \"$MANIFEST\" > /tmp/manifest.yaml\n\
             grep -q \"image: ${pinned}\\$\" /tmp/manifest.yaml || { echo \"$MANIFEST never names $IMAGE\" >&2; exit 1; }\n\
             kubectl apply -f /tmp/manifest.yaml\n",
        ))],
    )
}

// ---------------------------------------------------------------------------
// Builders

#[allow(clippy::too_many_arguments)]
fn task(
    name: &str,
    description: &str,
    params: Vec<Y>,
    workspaces: Vec<Y>,
    results: Vec<Y>,
    volumes: Vec<Y>,
    steps: Vec<Y>,
) -> Y {
    let mut spec = Mapping::new();
    spec.insert(s("description"), s(description));
    spec.insert(s("params"), Y::Sequence(params));
    spec.insert(s("workspaces"), Y::Sequence(workspaces));
    if !results.is_empty() {
        spec.insert(s("results"), Y::Sequence(results));
    }
    if !volumes.is_empty() {
        spec.insert(s("volumes"), Y::Sequence(volumes));
    }
    spec.insert(s("steps"), Y::Sequence(steps));
    super::object(
        "Task",
        name,
        Mapping::new(),
        Mapping::new(),
        Y::Mapping(spec),
    )
}

fn param(name: &str, description: &str) -> Y {
    map([
        ("name", s(name)),
        ("type", s("string")),
        ("description", s(description)),
    ])
}

/// Every runner's trailing argv: task vars, script arguments, nx overrides.
fn args_param() -> Y {
    map([
        ("name", s("args")),
        ("type", s("array")),
        ("description", s("Extra arguments after the target.")),
        ("default", Y::Sequence(Vec::new())),
    ])
}

fn workspace(name: &str, description: &str) -> Y {
    map([("name", s(name)), ("description", s(description))])
}

fn env(pairs: &[(&str, &str)]) -> Y {
    Y::Sequence(
        pairs
            .iter()
            .map(|(k, v)| map([("name", s(k)), ("value", s(v))]))
            .collect(),
    )
}

/// What every step starts with. `computeResources: {}` is the admission
/// webhook's own default: spelled out, a re-apply finds nothing to patch.
fn step_base(name: &str, image: &str, vars: &[(&str, &str)]) -> Mapping {
    let mut step = Mapping::new();
    step.insert(s("name"), s(name));
    step.insert(s("image"), s(image));
    step.insert(s("computeResources"), Y::Mapping(Mapping::new()));
    if !vars.is_empty() {
        step.insert(s("env"), env(vars));
    }
    step
}

fn script_step(name: &str, image: &str, vars: &[(&str, &str)], script: &str) -> Y {
    let mut step = step_base(name, image, vars);
    step.insert(s("script"), s(script));
    Y::Mapping(step)
}

/// A step that execs the tool directly, the `args` param appended to argv.
fn argv_step(name: &str, image: &str, vars: &[(&str, &str)], command: &[&str]) -> Y {
    let mut step = step_base(name, image, vars);
    step.insert(
        s("command"),
        Y::Sequence(command.iter().map(|c| s(c)).collect()),
    );
    step.insert(s("args"), Y::Sequence(vec![s("$(params.args[*])")]));
    Y::Mapping(step)
}

fn in_source(mut step: Y) -> Y {
    set(&mut step, "workingDir", s("$(workspaces.source.path)"));
    step
}

fn with_bin(mut step: Y) -> Y {
    set(
        &mut step,
        "volumeMounts",
        Y::Sequence(vec![map([
            ("name", s(BIN_VOLUME)),
            ("mountPath", s(BIN_DIR)),
        ])]),
    );
    step
}

fn set(step: &mut Y, key: &str, value: Y) {
    if let Y::Mapping(m) = step {
        m.insert(s(key), value);
    }
}
