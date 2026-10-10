//! Minimal GitHub REST client: exactly the endpoints the sync reads.
//!
//! Token auth, `Accept: application/vnd.github+json`, a pinned
//! `X-GitHub-Api-Version`, `Link`-header pagination. Rate limits are not
//! retried here: a 403/429 carrying `x-ratelimit-remaining: 0` or
//! `retry-after` becomes [`GitHubError::RateLimited`] with the reset time, and
//! the sync stops talking to GitHub until then.

use chrono::{DateTime, Duration, Utc};
use reqwest::header::{ACCEPT, HeaderMap, LINK};
use reqwest::{Client, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;

pub const API_URL: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
/// The listing endpoints' maximum page size.
const PER_PAGE: u32 = 100;
/// GitHub caps one filtered run listing at 1000 results, so a backfill lists
/// the window in slices small enough never to reach it.
const RUN_LISTING_SLICE: Duration = Duration::days(7);

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("GitHub rate limit reached; blocked until {reset_at}")]
    RateLimited { reset_at: DateTime<Utc> },
    #[error("GitHub {status} for {url}: {body}")]
    Status {
        status: u16,
        url: String,
        body: String,
    },
    #[error("GitHub request {url}: {source}")]
    Transport {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("GitHub response {url}: {source}")]
    Decode {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

impl GitHubError {
    /// The resource is not there and asking again will not change that
    /// (deleted commit, expired artifact, purged logs).
    pub fn is_gone(&self) -> bool {
        matches!(
            self,
            Self::Status {
                status: 404 | 410 | 422,
                ..
            }
        )
    }
}

pub type GitHubResult<T> = Result<T, GitHubError>;

#[derive(Clone)]
pub struct GitHub {
    http: Client,
    api: String,
    repo: String,
    token: String,
}

impl GitHub {
    /// `repo` is `owner/name`.
    pub fn new(token: impl Into<String>, repo: impl Into<String>) -> GitHubResult<Self> {
        let http = Client::builder()
            .user_agent(concat!("taskgraph-insights/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|source| GitHubError::Transport {
                url: API_URL.to_string(),
                source,
            })?;
        Ok(Self {
            http,
            api: API_URL.to_string(),
            repo: repo.into(),
            token: token.into(),
        })
    }

    pub fn repository(&self) -> &str {
        &self.repo
    }

    fn repo_url(&self, path: &str) -> String {
        if path.is_empty() {
            format!("{}/repos/{}", self.api, self.repo)
        } else {
            format!("{}/repos/{}/{path}", self.api, self.repo)
        }
    }

    async fn send(&self, url: &str) -> GitHubResult<Response> {
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .send()
            .await
            .map_err(|source| GitHubError::Transport {
                url: url.to_string(),
                source,
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        if let Some(reset_at) = rate_limit_reset(status.as_u16(), response.headers(), Utc::now()) {
            return Err(GitHubError::RateLimited { reset_at });
        }
        let mut body = response.text().await.unwrap_or_default();
        body.truncate(500);
        Err(GitHubError::Status {
            status: status.as_u16(),
            url: url.to_string(),
            body,
        })
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> GitHubResult<(T, Option<String>)> {
        let response = self.send(url).await?;
        let next = response
            .headers()
            .get(LINK)
            .and_then(|v| v.to_str().ok())
            .and_then(next_link);
        let bytes = response
            .bytes()
            .await
            .map_err(|source| GitHubError::Transport {
                url: url.to_string(),
                source,
            })?;
        let value = serde_json::from_slice(&bytes).map_err(|source| GitHubError::Decode {
            url: url.to_string(),
            source,
        })?;
        Ok((value, next))
    }

    /// Every page, following `Link: <…>; rel="next"`.
    async fn get_pages<P: Page>(&self, first: String) -> GitHubResult<Vec<P::Item>> {
        let mut items = Vec::new();
        let mut url = Some(first);
        while let Some(current) = url {
            let (page, next): (P, _) = self.get_json(&current).await?;
            items.extend(page.into_items());
            url = next;
        }
        Ok(items)
    }

    pub async fn default_branch(&self) -> GitHubResult<String> {
        #[derive(Deserialize)]
        struct Repo {
            default_branch: String,
        }
        let (repo, _): (Repo, _) = self.get_json(&self.repo_url("")).await?;
        Ok(repo.default_branch)
    }

    /// Runs created in `[since, until]`, each as of its latest attempt,
    /// deduplicated by id (slice boundaries are inclusive on both ends).
    pub async fn workflow_runs(
        &self,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> GitHubResult<Vec<WorkflowRun>> {
        let mut runs: Vec<WorkflowRun> = Vec::new();
        let mut from = since;
        while from <= until {
            let to = (from + RUN_LISTING_SLICE).min(until);
            let url = self.repo_url(&format!(
                "actions/runs?per_page={PER_PAGE}&created={}..{}",
                github_time(from),
                github_time(to)
            ));
            for run in self.get_pages::<RunsPage>(url).await? {
                if !runs.iter().any(|r| r.id == run.id) {
                    runs.push(run);
                }
            }
            if to == until {
                break;
            }
            from = to;
        }
        Ok(runs)
    }

    pub async fn run_attempt(&self, run_id: i64, attempt: i32) -> GitHubResult<WorkflowRun> {
        let url = self.repo_url(&format!("actions/runs/{run_id}/attempts/{attempt}"));
        Ok(self.get_json(&url).await?.0)
    }

    /// Jobs of one attempt, each with its steps.
    pub async fn attempt_jobs(&self, run_id: i64, attempt: i32) -> GitHubResult<Vec<Job>> {
        let url = self.repo_url(&format!(
            "actions/runs/{run_id}/attempts/{attempt}/jobs?per_page={PER_PAGE}"
        ));
        self.get_pages::<JobsPage>(url).await
    }

    /// Artifacts of every attempt of a run.
    pub async fn run_artifacts(&self, run_id: i64) -> GitHubResult<Vec<Artifact>> {
        let url = self.repo_url(&format!(
            "actions/runs/{run_id}/artifacts?per_page={PER_PAGE}"
        ));
        self.get_pages::<ArtifactsPage>(url).await
    }

    /// The artifact's zip archive. GitHub answers with a redirect to blob
    /// storage; reqwest follows it and drops the `Authorization` header on
    /// the cross-host hop.
    pub async fn download_artifact(&self, artifact_id: i64) -> GitHubResult<Vec<u8>> {
        let url = self.repo_url(&format!("actions/artifacts/{artifact_id}/zip"));
        let response = self.send(&url).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|source| GitHubError::Transport { url, source })?;
        Ok(bytes.to_vec())
    }

    /// Commits reachable from `branch` with a committer date at/after `since`.
    pub async fn commits(&self, branch: &str, since: DateTime<Utc>) -> GitHubResult<Vec<Commit>> {
        let url = self.repo_url(&format!(
            "commits?sha={}&since={}&per_page={PER_PAGE}",
            encode_query_value(branch),
            github_time(since)
        ));
        self.get_pages::<Vec<Commit>>(url).await
    }

    /// One commit with `stats` and (the first page of) `files`.
    pub async fn commit(&self, sha: &str) -> GitHubResult<Commit> {
        let url = self.repo_url(&format!("commits/{}", encode_query_value(sha)));
        Ok(self.get_json(&url).await?.0)
    }
}

/// When a failed response means "rate limited", the moment asking again makes
/// sense. `retry-after` (secondary limits) wins over the primary limit's
/// `x-ratelimit-reset`; a bare 429 waits a minute.
pub fn rate_limit_reset(
    status: u16,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if status != 403 && status != 429 {
        return None;
    }
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    if let Some(secs) = header("retry-after").and_then(|v| v.parse::<i64>().ok()) {
        return Some(now + Duration::seconds(secs.max(1)));
    }
    if header("x-ratelimit-remaining") == Some("0") {
        let reset = header("x-ratelimit-reset")
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|epoch| DateTime::from_timestamp(epoch, 0));
        return Some(reset.unwrap_or(now + Duration::minutes(1)));
    }
    (status == 429).then(|| now + Duration::minutes(1))
}

/// The `rel="next"` target of an RFC 8288 `Link` header, as GitHub sends it:
/// `<url>; rel="next", <url>; rel="last"`.
pub fn next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let (target, params) = part.split_once(';')?;
        let is_next = params.split(';').any(|p| {
            p.trim().strip_prefix("rel=").is_some_and(|rel| {
                rel.trim_matches('"')
                    .split_whitespace()
                    .any(|r| r == "next")
            })
        });
        let target = target.trim();
        (is_next && target.starts_with('<') && target.ends_with('>'))
            .then(|| target[1..target.len() - 1].to_string())
    })
}

fn github_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Percent-encode everything but RFC 3986 unreserved characters.
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A listing response: a bare array or an envelope around one.
trait Page: DeserializeOwned {
    type Item;
    fn into_items(self) -> Vec<Self::Item>;
}

impl<T: DeserializeOwned> Page for Vec<T> {
    type Item = T;
    fn into_items(self) -> Vec<T> {
        self
    }
}

#[derive(Deserialize)]
struct RunsPage {
    workflow_runs: Vec<WorkflowRun>,
}

impl Page for RunsPage {
    type Item = WorkflowRun;
    fn into_items(self) -> Vec<WorkflowRun> {
        self.workflow_runs
    }
}

#[derive(Deserialize)]
struct JobsPage {
    jobs: Vec<Job>,
}

impl Page for JobsPage {
    type Item = Job;
    fn into_items(self) -> Vec<Job> {
        self.jobs
    }
}

#[derive(Deserialize)]
struct ArtifactsPage {
    artifacts: Vec<Artifact>,
}

impl Page for ArtifactsPage {
    type Item = Artifact;
    fn into_items(self) -> Vec<Artifact> {
        self.artifacts
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub login: Option<String>,
}

/// A workflow run as of one attempt.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowRun {
    pub id: i64,
    pub name: Option<String>,
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub path: String,
    pub run_number: i64,
    pub event: String,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub workflow_id: i64,
    pub html_url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub run_attempt: Option<i32>,
    pub run_started_at: Option<DateTime<Utc>>,
    pub actor: Option<Account>,
    pub triggering_actor: Option<Account>,
}

impl WorkflowRun {
    pub fn attempt(&self) -> i32 {
        self.run_attempt.unwrap_or(1)
    }

    pub fn is_completed(&self) -> bool {
        self.status.as_deref() == Some("completed")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub id: i64,
    pub run_id: i64,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub runner_name: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub html_url: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub number: i32,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    pub id: i64,
    pub name: String,
    pub size_in_bytes: i64,
    pub expired: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Commit {
    pub sha: String,
    pub commit: GitCommit,
    /// The GitHub account linked to the author email, when there is one.
    pub author: Option<Account>,
    pub html_url: Option<String>,
    /// Only on the single-commit endpoint.
    pub stats: Option<CommitStats>,
    pub files: Option<Vec<CommitFile>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitCommit {
    pub author: Signature,
    pub committer: Signature,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Signature {
    pub name: String,
    pub email: String,
    pub date: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitStats {
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitFile {
    pub filename: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub previous_filename: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn next_link_follows_rel_next_only() {
        let header = r#"<https://api.github.com/repositories/1/actions/runs?per_page=100&page=2>; rel="next", <https://api.github.com/repositories/1/actions/runs?per_page=100&page=9>; rel="last""#;
        assert_eq!(
            next_link(header).as_deref(),
            Some("https://api.github.com/repositories/1/actions/runs?per_page=100&page=2")
        );
        // Last page: only prev/first remain, so pagination stops.
        let last = r#"<https://api.github.com/x?page=8>; rel="prev", <https://api.github.com/x?page=1>; rel="first""#;
        assert_eq!(next_link(last), None);
        // Order and whitespace are not significant.
        let reordered = r#"<https://h/x?page=1>; rel="first",<https://h/x?page=3>;rel="next""#;
        assert_eq!(next_link(reordered).as_deref(), Some("https://h/x?page=3"));
    }

    #[test]
    fn rate_limit_reset_reads_retry_after_before_the_primary_reset() {
        let now = DateTime::from_timestamp(1_790_000_000, 0).expect("valid");
        let mut primary = HeaderMap::new();
        primary.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        primary.insert("x-ratelimit-reset", HeaderValue::from_static("1790000900"));
        assert_eq!(
            rate_limit_reset(403, &primary, now),
            DateTime::from_timestamp(1_790_000_900, 0)
        );

        let mut secondary = primary.clone();
        secondary.insert("retry-after", HeaderValue::from_static("30"));
        assert_eq!(
            rate_limit_reset(403, &secondary, now),
            Some(now + Duration::seconds(30))
        );

        // A 403 with quota left is a permission problem, not a rate limit.
        let mut forbidden = HeaderMap::new();
        forbidden.insert("x-ratelimit-remaining", HeaderValue::from_static("4999"));
        assert_eq!(rate_limit_reset(403, &forbidden, now), None);
        assert_eq!(rate_limit_reset(500, &primary, now), None);
    }
}
