//! Commit attribution, via the shared `core_authorship` registry.

use chrono::{DateTime, Utc};
use core_authorship::{Attribution, CommitAuthor, ContributorKind, Registry, classify_commit};

use crate::github::Commit;

/// A commit ready to store.
#[derive(Debug, Clone)]
pub struct ClassifiedCommit {
    pub sha: String,
    pub authored_at: DateTime<Utc>,
    pub committed_at: DateTime<Utc>,
    pub author_login: Option<String>,
    pub author_name: String,
    pub author_email: String,
    /// Contributor id (`Classification::author`).
    pub author: String,
    pub author_kind: ContributorKind,
    /// Agent ids credited by trailers, never the author itself.
    pub assistants: Vec<String>,
    pub attribution: Attribution,
    pub message: String,
    pub subject: String,
    pub conv_type: Option<String>,
    pub html_url: Option<String>,
    /// Author and assistants as contributors: (id, kind, display name).
    pub contributors: Vec<(String, ContributorKind, String)>,
}

pub fn classify(registry: &Registry, commit: &Commit) -> ClassifiedCommit {
    let git = &commit.commit;
    let login = commit
        .author
        .as_ref()
        .and_then(|a| a.login.as_deref())
        .filter(|l| !l.is_empty());
    let classification = classify_commit(
        registry,
        CommitAuthor {
            login,
            name: &git.author.name,
            email: &git.author.email,
        },
        git.committer.date,
        &git.message,
    );
    let assistants: Vec<String> = classification
        .assistants
        .iter()
        .map(|a| a.agent.clone())
        .collect();

    let author_display = match classification.author_kind {
        ContributorKind::Agent => agent_name(registry, &classification.author),
        ContributorKind::Bot => classification.author.clone(),
        ContributorKind::Human => git.author.name.clone(),
    };
    let mut contributors = vec![(
        classification.author.clone(),
        classification.author_kind,
        author_display,
    )];
    contributors.extend(
        assistants
            .iter()
            .map(|id| (id.clone(), ContributorKind::Agent, agent_name(registry, id))),
    );

    let subject = git
        .message
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    ClassifiedCommit {
        sha: commit.sha.clone(),
        authored_at: git.author.date,
        committed_at: git.committer.date,
        author_login: login.map(str::to_string),
        author_name: git.author.name.clone(),
        author_email: git.author.email.clone(),
        author: classification.author,
        author_kind: classification.author_kind,
        assistants,
        attribution: classification.attribution,
        message: git.message.clone(),
        conv_type: conventional_type(&subject),
        subject,
        html_url: commit.html_url.clone(),
        contributors,
    }
}

fn agent_name(registry: &Registry, id: &str) -> String {
    registry
        .agent(id)
        .map_or_else(|| id.to_string(), |a| a.name.clone())
}

/// The Conventional Commits type of a subject (`fix(api)!: …` → `fix`),
/// lowercased; `None` for anything else (merges, reverts, prose).
pub fn conventional_type(subject: &str) -> Option<String> {
    let (head, _) = subject.split_once(':')?;
    let head = head.strip_suffix('!').unwrap_or(head);
    let kind = match head.split_once('(') {
        Some((kind, scope)) if scope.ends_with(')') && !scope[..scope.len() - 1].contains('(') => {
            kind
        }
        Some(_) => return None,
        None => head,
    };
    (!kind.is_empty() && kind.chars().all(|c| c.is_ascii_alphabetic()))
        .then(|| kind.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fixup_rate counts commits followed by a `fix`: the parser decides
    /// which commits are fixes.
    #[test]
    fn conventional_type_accepts_scopes_and_breaking_marks_only() {
        assert_eq!(conventional_type("fix: typo").as_deref(), Some("fix"));
        assert_eq!(
            conventional_type("Fix(api)!: drop v1").as_deref(),
            Some("fix")
        );
        assert_eq!(
            conventional_type("feat(ci/insights): x").as_deref(),
            Some("feat")
        );
        assert_eq!(conventional_type("Revert \"fix: typo\""), None);
        assert_eq!(conventional_type("Merge pull request #24 from x/y"), None);
        assert_eq!(conventional_type("fix typo: in readme"), None);
        assert_eq!(conventional_type("no colon at all"), None);
    }
}
