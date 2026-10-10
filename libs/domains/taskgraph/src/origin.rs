//! Where a run comes from: the CI job it ran in, who invoked go-task, and
//! the commit checked out.
//!
//! Environment access is injected (`get(name) -> Option<String>`), so the
//! detection is a pure function of the variables it is shown; an empty value
//! counts as unset.
//!
//! CI detection reads the provider's own variables, then applies the explicit
//! `TASKGRAPH_CI_*` overrides field by field. The overrides are how a system
//! with no auto-detection (Tekton) or a job that knows better (a
//! `pull_request` job passing the PR head sha) describes itself;
//! `TASKGRAPH_CI_PROVIDER` + `TASKGRAPH_CI_RUN_ID` alone are enough to mark a
//! run as CI. A detection without a provider and a run id is no detection.

use std::path::Path;
use std::process::{Command, Stdio};

use contract_taskgraph::{CiContext, Invoker, InvokerKind, RunOrigin};
use core_authorship::{is_truthy, registry};

/// Explicit per-field overrides, in [`CiContext`] field order.
pub const CI_OVERRIDES: [&str; 11] = [
    "TASKGRAPH_CI_PROVIDER",
    "TASKGRAPH_CI_RUN_ID",
    "TASKGRAPH_CI_RUN_ATTEMPT",
    "TASKGRAPH_CI_RUN_URL",
    "TASKGRAPH_CI_PIPELINE",
    "TASKGRAPH_CI_JOB",
    "TASKGRAPH_CI_REPOSITORY",
    "TASKGRAPH_CI_REF",
    "TASKGRAPH_CI_SHA",
    "TASKGRAPH_CI_EVENT",
    "TASKGRAPH_CI_ACTOR",
];

/// The CI job this process runs in, if any.
pub fn detect_ci(get: impl Fn(&str) -> Option<String>) -> Option<CiContext> {
    let var = |name: &str| get(name).filter(|v| !v.trim().is_empty());
    let flag = |name: &str| var(name).is_some_and(|v| is_truthy(&v));

    let mut ci = if flag("GITHUB_ACTIONS") {
        github(&var)
    } else if flag("GITLAB_CI") {
        gitlab(&var)
    } else if flag("BUILDKITE") {
        buildkite(&var)
    } else if flag("CIRCLECI") {
        circleci(&var)
    } else if var("JENKINS_URL").is_some() {
        jenkins(&var)
    } else {
        CiContext::default()
    };

    let [
        provider,
        run_id,
        attempt,
        run_url,
        pipeline,
        job,
        repository,
        git_ref,
        sha,
        event,
        actor,
    ] = CI_OVERRIDES.map(var);
    if let Some(v) = provider {
        ci.provider = v;
    }
    if let Some(v) = run_id {
        ci.run_id = v;
    }
    if let Some(v) = attempt.and_then(|a| a.trim().parse().ok()) {
        ci.run_attempt = Some(v);
    }
    for (slot, value) in [
        (&mut ci.run_url, run_url),
        (&mut ci.pipeline, pipeline),
        (&mut ci.job, job),
        (&mut ci.repository, repository),
        (&mut ci.git_ref, git_ref),
        (&mut ci.sha, sha),
        (&mut ci.event, event),
        (&mut ci.actor, actor),
    ] {
        if value.is_some() {
            *slot = value;
        }
    }

    (!ci.provider.is_empty() && !ci.run_id.is_empty()).then_some(ci)
}

/// Origin of a run started now in `cwd`.
pub fn detect_origin(get: impl Fn(&str) -> Option<String>, cwd: &Path) -> RunOrigin {
    let ci = detect_ci(&get);
    let invoker = detect_invoker(&get, ci.is_some());
    RunOrigin {
        ci,
        invoker: Some(invoker),
        sha: git_head(cwd),
    }
}

/// Agent (a registry marker is set) > CI (a detected job, or the generic
/// `CI` marker nearly every CI system exports) > human.
pub fn detect_invoker(get: impl Fn(&str) -> Option<String>, in_ci_job: bool) -> Invoker {
    if let Some(agent) = registry().detect_agent(&get) {
        return Invoker {
            kind: InvokerKind::Agent,
            agent: Some(agent.to_string()),
        };
    }
    let generic_ci = get("CI").is_some_and(|v| is_truthy(&v));
    Invoker {
        kind: if in_ci_job || generic_ci {
            InvokerKind::Ci
        } else {
            InvokerKind::Human
        },
        agent: None,
    }
}

/// `git rev-parse HEAD` in `dir`; `None` outside a repository, in one with no
/// commits yet, or without git on PATH.
pub fn git_head(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

fn github(var: &impl Fn(&str) -> Option<String>) -> CiContext {
    let run_id = var("GITHUB_RUN_ID").unwrap_or_default();
    let repository = var("GITHUB_REPOSITORY");
    let run_url = repository
        .as_ref()
        .filter(|_| !run_id.is_empty())
        .map(|repo| {
            let server = var("GITHUB_SERVER_URL").unwrap_or_else(|| "https://github.com".into());
            format!(
                "{}/{repo}/actions/runs/{run_id}",
                server.trim_end_matches('/')
            )
        });
    CiContext {
        provider: "github_actions".into(),
        run_id,
        run_attempt: var("GITHUB_RUN_ATTEMPT").and_then(|a| a.parse().ok()),
        run_url,
        pipeline: var("GITHUB_WORKFLOW"),
        job: var("GITHUB_JOB"),
        repository,
        git_ref: var("GITHUB_REF"),
        // On `pull_request` this is the merge commit; the job passes the PR
        // head as TASKGRAPH_CI_SHA, which wins below.
        sha: var("GITHUB_SHA"),
        event: var("GITHUB_EVENT_NAME"),
        actor: var("GITHUB_ACTOR"),
    }
}

fn gitlab(var: &impl Fn(&str) -> Option<String>) -> CiContext {
    CiContext {
        provider: "gitlab_ci".into(),
        run_id: var("CI_PIPELINE_ID").unwrap_or_default(),
        run_attempt: None,
        run_url: var("CI_PIPELINE_URL"),
        pipeline: var("CI_PIPELINE_NAME"),
        job: var("CI_JOB_NAME"),
        repository: var("CI_PROJECT_PATH"),
        git_ref: var("CI_COMMIT_REF_NAME"),
        sha: var("CI_COMMIT_SHA"),
        event: var("CI_PIPELINE_SOURCE"),
        actor: var("GITLAB_USER_LOGIN"),
    }
}

fn buildkite(var: &impl Fn(&str) -> Option<String>) -> CiContext {
    CiContext {
        provider: "buildkite".into(),
        run_id: var("BUILDKITE_BUILD_ID").unwrap_or_default(),
        // Retries are per job and counted from 0.
        run_attempt: var("BUILDKITE_RETRY_COUNT")
            .and_then(|n| n.parse::<u32>().ok())
            .map(|n| n.saturating_add(1)),
        run_url: var("BUILDKITE_BUILD_URL"),
        pipeline: var("BUILDKITE_PIPELINE_SLUG"),
        job: var("BUILDKITE_STEP_KEY").or_else(|| var("BUILDKITE_LABEL")),
        repository: var("BUILDKITE_REPO"),
        git_ref: var("BUILDKITE_BRANCH"),
        sha: var("BUILDKITE_COMMIT"),
        event: var("BUILDKITE_SOURCE"),
        actor: var("BUILDKITE_BUILD_CREATOR"),
    }
}

fn circleci(var: &impl Fn(&str) -> Option<String>) -> CiContext {
    let repository = match (
        var("CIRCLE_PROJECT_USERNAME"),
        var("CIRCLE_PROJECT_REPONAME"),
    ) {
        (Some(owner), Some(name)) => Some(format!("{owner}/{name}")),
        _ => None,
    };
    CiContext {
        provider: "circleci".into(),
        run_id: var("CIRCLE_WORKFLOW_ID").unwrap_or_default(),
        run_attempt: None,
        run_url: var("CIRCLE_BUILD_URL"),
        pipeline: None,
        job: var("CIRCLE_JOB"),
        repository,
        git_ref: var("CIRCLE_TAG").or_else(|| var("CIRCLE_BRANCH")),
        sha: var("CIRCLE_SHA1"),
        event: None,
        actor: var("CIRCLE_USERNAME"),
    }
}

fn jenkins(var: &impl Fn(&str) -> Option<String>) -> CiContext {
    CiContext {
        provider: "jenkins".into(),
        // `jenkins-<job>-<number>`: unique across jobs, unlike BUILD_NUMBER.
        run_id: var("BUILD_TAG").unwrap_or_default(),
        run_attempt: None,
        run_url: var("BUILD_URL"),
        pipeline: var("JOB_NAME"),
        job: var("STAGE_NAME"),
        repository: var("GIT_URL"),
        git_ref: var("GIT_BRANCH"),
        sha: var("GIT_COMMIT"),
        event: None,
        actor: var("BUILD_USER_ID"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    const GITHUB: &[(&str, &str)] = &[
        ("GITHUB_ACTIONS", "true"),
        ("GITHUB_RUN_ID", "123"),
        ("GITHUB_RUN_ATTEMPT", "2"),
        ("GITHUB_SERVER_URL", "https://github.com/"),
        ("GITHUB_REPOSITORY", "org/repo"),
        ("GITHUB_WORKFLOW", "CI"),
        ("GITHUB_JOB", "rust"),
        ("GITHUB_REF", "refs/pull/7/merge"),
        ("GITHUB_SHA", "merge-sha"),
        ("GITHUB_EVENT_NAME", "pull_request"),
        ("GITHUB_ACTOR", "octocat"),
    ];

    #[test]
    fn github_actions_is_read_in_full() {
        let ci = detect_ci(env(GITHUB)).expect("detected");
        assert_eq!(
            ci,
            CiContext {
                provider: "github_actions".into(),
                run_id: "123".into(),
                run_attempt: Some(2),
                run_url: Some("https://github.com/org/repo/actions/runs/123".into()),
                pipeline: Some("CI".into()),
                job: Some("rust".into()),
                repository: Some("org/repo".into()),
                git_ref: Some("refs/pull/7/merge".into()),
                sha: Some("merge-sha".into()),
                event: Some("pull_request".into()),
                actor: Some("octocat".into()),
            }
        );
    }

    #[test]
    fn overrides_win_field_by_field_and_keep_the_rest() {
        let mut pairs = GITHUB.to_vec();
        pairs.extend([
            ("TASKGRAPH_CI_SHA", "pr-head-sha"),
            ("TASKGRAPH_CI_JOB", "rust (linux)"),
            ("TASKGRAPH_CI_RUN_ATTEMPT", "not-a-number"),
            // Empty means unset, not "clear the field".
            ("TASKGRAPH_CI_ACTOR", ""),
        ]);
        let ci = detect_ci(env(&pairs)).expect("detected");
        assert_eq!(ci.sha.as_deref(), Some("pr-head-sha"));
        assert_eq!(ci.job.as_deref(), Some("rust (linux)"));
        assert_eq!(ci.run_attempt, Some(2));
        assert_eq!(ci.actor.as_deref(), Some("octocat"));
        assert_eq!(ci.provider, "github_actions");
    }

    #[test]
    fn provider_and_run_id_overrides_alone_mark_ci() {
        let ci = detect_ci(env(&[
            ("TASKGRAPH_CI_PROVIDER", "tekton"),
            ("TASKGRAPH_CI_RUN_ID", "ci-run-abc"),
            ("TASKGRAPH_CI_JOB", "build"),
        ]))
        .expect("detected");
        assert_eq!(ci.provider, "tekton");
        assert_eq!(ci.run_id, "ci-run-abc");
        assert_eq!(ci.job.as_deref(), Some("build"));
    }

    #[test]
    fn incomplete_identity_is_not_ci() {
        // A job name without a provider/run id identifies nothing.
        assert_eq!(detect_ci(env(&[("TASKGRAPH_CI_JOB", "build")])), None);
        assert_eq!(detect_ci(env(&[("TASKGRAPH_CI_PROVIDER", "tekton")])), None);
        // `GITHUB_ACTIONS=false` is not GitHub.
        assert_eq!(
            detect_ci(env(&[("GITHUB_ACTIONS", "false"), ("GITHUB_RUN_ID", "1")])),
            None
        );
        assert_eq!(detect_ci(env(&[])), None);
    }

    #[test]
    fn other_providers_map_their_run_identity() {
        let gitlab = detect_ci(env(&[
            ("GITLAB_CI", "true"),
            ("CI_PIPELINE_ID", "77"),
            ("CI_JOB_NAME", "test"),
            ("CI_PROJECT_PATH", "g/p"),
        ]))
        .expect("gitlab");
        assert_eq!(
            (gitlab.provider.as_str(), gitlab.run_id.as_str()),
            ("gitlab_ci", "77")
        );
        assert_eq!(gitlab.repository.as_deref(), Some("g/p"));

        let buildkite = detect_ci(env(&[
            ("BUILDKITE", "true"),
            ("BUILDKITE_BUILD_ID", "b-1"),
            ("BUILDKITE_RETRY_COUNT", "0"),
        ]))
        .expect("buildkite");
        assert_eq!(buildkite.run_attempt, Some(1));

        let circle = detect_ci(env(&[
            ("CIRCLECI", "true"),
            ("CIRCLE_WORKFLOW_ID", "w-1"),
            ("CIRCLE_PROJECT_USERNAME", "o"),
            ("CIRCLE_PROJECT_REPONAME", "r"),
        ]))
        .expect("circleci");
        assert_eq!(circle.repository.as_deref(), Some("o/r"));

        let jenkins = detect_ci(env(&[
            ("JENKINS_URL", "https://ci.example/"),
            ("BUILD_TAG", "jenkins-app-5"),
        ]))
        .expect("jenkins");
        assert_eq!(jenkins.run_id, "jenkins-app-5");
    }

    #[test]
    fn invoker_precedence_is_agent_then_ci_then_human() {
        let agent_in_ci = env(&[("GITHUB_ACTIONS", "true"), ("CLAUDECODE", "1")]);
        assert_eq!(
            detect_invoker(&agent_in_ci, true),
            Invoker {
                kind: InvokerKind::Agent,
                agent: Some("claude-code".into())
            }
        );
        assert_eq!(
            detect_invoker(env(&[]), true).kind,
            InvokerKind::Ci,
            "a detected job is CI"
        );
        assert_eq!(
            detect_invoker(env(&[("CI", "true")]), false).kind,
            InvokerKind::Ci,
            "the generic marker is CI"
        );
        assert_eq!(
            detect_invoker(env(&[("CI", "false"), ("CLAUDECODE", "0")]), false),
            Invoker {
                kind: InvokerKind::Human,
                agent: None
            }
        );
    }

    #[test]
    fn origin_outside_a_repository_has_no_sha() {
        let dir = std::env::temp_dir().join(format!("taskgraph-origin-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let origin = detect_origin(env(&[("CI", "1")]), &dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(origin.sha, None);
        assert_eq!(origin.ci, None);
        assert_eq!(origin.invoker.map(|i| i.kind), Some(InvokerKind::Ci));
    }
}
