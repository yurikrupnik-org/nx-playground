//! Who wrote a change, and who ran a command: a person, an AI coding agent, or
//! automation.
//!
//! `agents.json` (next to this crate's `Cargo.toml`) is the single registry.
//! The git hooks and the CI check in `tools/authorship/` read the same file,
//! so the rules below and the hook that stamps commits cannot drift apart.
//!
//! - **Runtime**: [`Registry::detect_agent`] reads the environment markers AI
//!   coding agents export into the shells they spawn (`CLAUDECODE`,
//!   `CODEX_SESSION_ID`, `GEMINI_CLI`, …).
//! - **History**: [`classify_commit`] turns a commit's author identity and its
//!   `Assisted-by:` / `Co-authored-by:` trailers into a [`Classification`].
//!
//! Trailers are only trustworthy from the day the hooks started stamping them
//! ([`Registry::enforced_since`]); older commits are [`Attribution::Legacy`]:
//! "no trailer" there means "unknown", not "human".

use std::sync::LazyLock;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;

const REGISTRY_JSON: &str = include_str!("../agents.json");

/// GitHub's noreply domain for app/bot accounts (`name[bot]@users.noreply.github.com`).
const BOT_EMAIL_SUFFIX: &str = "[bot]@users.noreply.github.com";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    /// Trailer key the hooks stamp (`Assisted-by`).
    pub trailer: String,
    /// First day on which agent commits are stamped by the hooks.
    pub enforced_since: NaiveDate,
    /// Markers that say "some agent" without saying which.
    pub generic_env: Vec<String>,
    /// Agent id reported for [`Self::generic_env`] matches.
    pub generic_agent_id: String,
    /// Detection order: the first agent with a marker set wins.
    pub agents: Vec<Agent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    /// Environment variables the agent exports into the shells it spawns.
    pub env: Vec<String>,
    /// Git author names / GitHub logins the agent commits as.
    pub identities: Vec<String>,
    /// Emails the agent's `Co-authored-by:` trailers carry.
    pub coauthor_emails: Vec<String>,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    serde_json::from_str(REGISTRY_JSON)
        .expect("agents.json is valid; `registry_is_well_formed` guards it")
});

/// The registry compiled into this binary.
pub fn registry() -> &'static Registry {
    &REGISTRY
}

/// A marker counts when it is set to anything but empty, `0` or `false`.
pub fn is_truthy(value: &str) -> bool {
    let v = value.trim();
    !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false"))
}

impl Registry {
    pub fn agent(&self, id: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == id)
    }

    /// Whether `id` is a registered agent or the generic fallback id.
    pub fn is_known_agent(&self, id: &str) -> bool {
        id == self.generic_agent_id || self.agent(id).is_some()
    }

    /// The agent whose markers are set in the environment `get` reads, in
    /// registry order; the generic id when only a generic marker is set.
    pub fn detect_agent(&self, get: impl Fn(&str) -> Option<String>) -> Option<&str> {
        let set = |var: &String| get(var).is_some_and(|v| is_truthy(&v));
        self.agents
            .iter()
            .find(|a| a.env.iter().any(set))
            .map(|a| a.id.as_str())
            .or_else(|| {
                self.generic_env
                    .iter()
                    .any(set)
                    .then_some(self.generic_agent_id.as_str())
            })
    }

    /// The agent that commits as `identity` (a git author name or GitHub login).
    pub fn agent_for_identity(&self, identity: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| {
            a.identities
                .iter()
                .any(|i| i.eq_ignore_ascii_case(identity.trim()))
        })
    }

    /// The agent whose `Co-authored-by:` trailers carry `email`.
    pub fn agent_for_email(&self, email: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| {
            a.coauthor_emails
                .iter()
                .any(|e| e.eq_ignore_ascii_case(email.trim()))
        })
    }
}

/// One `Key: value` git trailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trailer {
    pub key: String,
    pub value: String,
}

/// The trailers of a commit message: its last paragraph, when every line of
/// it is `Key: value` (a line starting with whitespace continues the previous
/// value). This is the common case of `git interpret-trailers --parse`; a
/// last paragraph that is prose yields nothing.
pub fn trailers(message: &str) -> Vec<Trailer> {
    let body = message.trim_end();
    let Some(last) = body.rsplit("\n\n").next() else {
        return Vec::new();
    };
    // A subject line alone is not a trailer block.
    if last.len() == body.len() && !body.contains('\n') {
        return Vec::new();
    }
    let mut out: Vec<Trailer> = Vec::new();
    for line in last.lines() {
        if line.starts_with([' ', '\t']) {
            match out.last_mut() {
                Some(prev) => {
                    prev.value.push(' ');
                    prev.value.push_str(line.trim());
                }
                None => return Vec::new(),
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Vec::new();
        };
        let key_ok = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !key_ok || value.trim().is_empty() {
            return Vec::new();
        }
        out.push(Trailer {
            key: key.to_string(),
            value: value.trim().to_string(),
        });
    }
    out
}

/// An agent credited on a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assist {
    /// Registry id when recognised; otherwise the value as written, lowercased.
    pub agent: String,
    /// `Assisted-by: <agent>:<model>` carries the model after the colon.
    pub model: Option<String>,
}

/// The `<email>` part of `Name <email>`.
fn trailer_email(value: &str) -> Option<&str> {
    let start = value.rfind('<')?;
    let end = value[start..].find('>')? + start;
    Some(value[start + 1..end].trim())
}

impl Registry {
    /// Agents credited by `message`: every `Assisted-by:` trailer, plus every
    /// `Co-authored-by:` whose email belongs to a registered agent. First
    /// mention wins; an agent is listed once.
    pub fn assists(&self, message: &str) -> Vec<Assist> {
        let mut out: Vec<Assist> = Vec::new();
        let mut push = |assist: Assist| {
            if !out.iter().any(|a| a.agent == assist.agent) {
                out.push(assist);
            }
        };
        for t in trailers(message) {
            if t.key.eq_ignore_ascii_case(&self.trailer) {
                let (agent, model) = match t.value.split_once(':') {
                    Some((a, m)) if !m.trim().is_empty() => (a, Some(m.trim().to_string())),
                    Some((a, _)) => (a, None),
                    None => (t.value.as_str(), None),
                };
                push(Assist {
                    agent: agent.trim().to_ascii_lowercase(),
                    model,
                });
            } else if t.key.eq_ignore_ascii_case("Co-authored-by")
                && let Some(agent) = trailer_email(&t.value).and_then(|e| self.agent_for_email(e))
            {
                push(Assist {
                    agent: agent.id.clone(),
                    model: None,
                });
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContributorKind {
    Human,
    Agent,
    Bot,
}

impl ContributorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Bot => "bot",
        }
    }
}

/// Whether a commit's trailers are trustworthy (see the crate docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Attribution {
    /// Committed on or after [`Registry::enforced_since`].
    Enforced,
    /// Committed before it: a missing trailer proves nothing.
    Legacy,
}

impl Attribution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforced => "enforced",
            Self::Legacy => "legacy",
        }
    }
}

/// A commit's author as git and GitHub report it.
#[derive(Debug, Clone, Copy)]
pub struct CommitAuthor<'a> {
    /// GitHub login, when the email is linked to an account.
    pub login: Option<&'a str>,
    pub name: &'a str,
    pub email: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// Stable contributor id: the agent id for an agent identity, the GitHub
    /// login when known, else the lowercased email.
    pub author: String,
    pub author_kind: ContributorKind,
    /// Agents credited alongside the author, never the author itself.
    pub assistants: Vec<Assist>,
    pub attribution: Attribution,
}

fn is_bot_identity(author: &CommitAuthor<'_>) -> bool {
    author.login.is_some_and(|l| l.ends_with("[bot]"))
        || author.name.ends_with("[bot]")
        || author
            .email
            .to_ascii_lowercase()
            .ends_with(BOT_EMAIL_SUFFIX)
}

/// Classify one commit.
///
/// Precedence: an identity registered to an agent is that agent (e.g.
/// `claude-maintenance[bot]` is `claude-code`, not a bot); any other `[bot]`
/// identity is a bot; everyone else is a human. Trailers never change the
/// author's kind — they add assistants.
pub fn classify_commit(
    registry: &Registry,
    author: CommitAuthor<'_>,
    committed_at: DateTime<Utc>,
    message: &str,
) -> Classification {
    let agent = author
        .login
        .and_then(|l| registry.agent_for_identity(l))
        .or_else(|| registry.agent_for_identity(author.name))
        .or_else(|| registry.agent_for_identity(author.email));
    let (id, kind) = match agent {
        Some(a) => (a.id.clone(), ContributorKind::Agent),
        None if is_bot_identity(&author) => (
            author
                .login
                .filter(|l| l.ends_with("[bot]"))
                .unwrap_or(author.name)
                .to_string(),
            ContributorKind::Bot,
        ),
        None => (
            author
                .login
                .map(str::to_string)
                .unwrap_or_else(|| author.email.trim().to_ascii_lowercase()),
            ContributorKind::Human,
        ),
    };
    let assistants = registry
        .assists(message)
        .into_iter()
        .filter(|a| a.agent != id)
        .collect();
    let attribution = if committed_at.date_naive() >= registry.enforced_since {
        Attribution::Enforced
    } else {
        Attribution::Legacy
    };
    Classification {
        author: id,
        author_kind: kind,
        assistants,
        attribution,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use chrono::TimeZone;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0)
            .single()
            .expect("valid date")
    }

    fn human<'a>() -> CommitAuthor<'a> {
        CommitAuthor {
            login: Some("yurikrupnik"),
            name: "Yuri Krupnik",
            email: "krupnik.yuri@gmail.com",
        }
    }

    #[test]
    fn registry_is_well_formed() {
        let r = registry();
        let mut ids = HashSet::new();
        for a in &r.agents {
            assert!(ids.insert(a.id.as_str()), "duplicate agent id {}", a.id);
            assert!(!a.env.is_empty(), "{} has no env marker", a.id);
            assert_eq!(a.id, a.id.to_ascii_lowercase(), "ids are lowercase");
            assert!(!a.id.contains(':'), "':' separates agent from model");
        }
        assert!(!r.is_known_agent("nobody"));
        assert!(r.is_known_agent(&r.generic_agent_id));
    }

    /// omp exports CLAUDECODE as well; the more specific marker must win.
    #[test]
    fn detection_follows_registry_order() {
        let r = registry();
        let get = env(&[("CLAUDECODE", "1"), ("OMPCODE", "1"), ("AGENT", "1")]);
        assert_eq!(r.detect_agent(get), Some("omp"));
        assert_eq!(
            r.detect_agent(env(&[("CLAUDECODE", "1")])),
            Some("claude-code")
        );
        assert_eq!(
            r.detect_agent(env(&[("CODEX_SANDBOX", "seatbelt")])),
            Some("codex")
        );
    }

    #[test]
    fn generic_markers_and_falsy_values() {
        let r = registry();
        assert_eq!(
            r.detect_agent(env(&[("AGENT", "1")])),
            Some("unknown-agent")
        );
        assert_eq!(r.detect_agent(env(&[("CLAUDECODE", "0")])), None);
        assert_eq!(r.detect_agent(env(&[("GEMINI_CLI", "")])), None);
        assert_eq!(r.detect_agent(env(&[("CURSOR_AGENT", "false")])), None);
        assert_eq!(r.detect_agent(env(&[])), None);
    }

    #[test]
    fn trailers_come_from_the_last_paragraph_only() {
        let msg = "feat(x): thing\n\nBody mentions Key: value in prose.\n\nAssisted-by: claude-code:claude-opus-5\nSigned-off-by: A <a@b>\n";
        let t = trailers(msg);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].key, "Assisted-by");
        assert_eq!(t[0].value, "claude-code:claude-opus-5");
        assert!(trailers("fix: subject only").is_empty());
        assert!(trailers("fix: x\n\nJust a closing sentence: really.\nAnd more prose").is_empty());
    }

    #[test]
    fn assists_merge_assisted_by_and_known_co_authors() {
        let r = registry();
        let msg = "feat: x\n\nAssisted-by: omp:claude-opus-5\nCo-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>\nCo-authored-by: Jane <jane@example.com>\nAssisted-by: omp\n";
        let got = r.assists(msg);
        assert_eq!(
            got,
            vec![
                Assist {
                    agent: "omp".into(),
                    model: Some("claude-opus-5".into())
                },
                Assist {
                    agent: "claude-code".into(),
                    model: None
                },
            ]
        );
    }

    #[test]
    fn agent_identities_are_agents_not_bots() {
        let r = registry();
        let c = classify_commit(
            r,
            CommitAuthor {
                login: Some("claude-maintenance[bot]"),
                name: "claude-maintenance[bot]",
                email: "claude-maintenance[bot]@users.noreply.github.com",
            },
            at(2026, 10, 1),
            "chore: bump\n\nAssisted-by: claude-code\n",
        );
        assert_eq!(c.author, "claude-code");
        assert_eq!(c.author_kind, ContributorKind::Agent);
        assert!(c.assistants.is_empty(), "an agent does not assist itself");
    }

    #[test]
    fn unregistered_bot_identities_are_bots() {
        let c = classify_commit(
            registry(),
            CommitAuthor {
                login: None,
                name: "github-actions[bot]",
                email: "github-actions[bot]@users.noreply.github.com",
            },
            at(2026, 10, 1),
            "chore(release): v1",
        );
        assert_eq!(c.author, "github-actions[bot]");
        assert_eq!(c.author_kind, ContributorKind::Bot);
    }

    #[test]
    fn humans_keep_their_kind_and_gain_assistants() {
        let r = registry();
        let c = classify_commit(
            r,
            human(),
            at(2026, 10, 1),
            "feat: x\n\nAssisted-by: codex\n",
        );
        assert_eq!(c.author, "yurikrupnik");
        assert_eq!(c.author_kind, ContributorKind::Human);
        assert_eq!(c.assistants.len(), 1);
        assert_eq!(c.assistants[0].agent, "codex");
        assert_eq!(c.attribution, Attribution::Enforced);

        let no_login = classify_commit(
            r,
            CommitAuthor {
                login: None,
                name: "Slava",
                email: "Slava@Example.com",
            },
            at(2026, 1, 1),
            "fix: y",
        );
        assert_eq!(no_login.author, "slava@example.com");
        assert_eq!(no_login.attribution, Attribution::Legacy);
    }

    #[test]
    fn enforcement_starts_on_the_registry_date() {
        let r = registry();
        let day = r.enforced_since;
        let on = Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"));
        let before = on - chrono::Duration::seconds(1);
        assert_eq!(
            classify_commit(r, human(), on, "x").attribution,
            Attribution::Enforced
        );
        assert_eq!(
            classify_commit(r, human(), before, "x").attribution,
            Attribution::Legacy
        );
    }
}
