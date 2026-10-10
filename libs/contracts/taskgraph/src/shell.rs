//! Static shell scan: the document `taskgraph shell-scan` writes and the
//! insights service ingests from CI artifacts.
//!
//! A scan covers every piece of shell this repository executes: script files
//! (`*.sh`, `*.bash`, extension-less files with a sh/bash shebang) and the
//! shell go-task runs from a Taskfile (`cmds:`, `status:`, `preconditions:`).
//! Each piece is a [`ShellSource`] with a stable id, so the same source can be
//! followed across scans; findings point back at it by that id.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::CiContext;

/// One scan of one checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellScan {
    pub scan_id: Uuid,
    pub scanned_at: DateTime<Utc>,
    /// `git rev-parse HEAD` of the scanned checkout, when it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// Linter that produced `findings`, with its version (`shellcheck 0.10.0`).
    pub tool: String,
    /// Set when the scan ran inside a CI job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ci: Option<CiContext>,
    /// Sorted by `id`.
    pub sources: Vec<ShellSource>,
    /// Sorted by (`source_id`, `line`, `column`, `code`).
    pub findings: Vec<ShellFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellSourceKind {
    /// A script file.
    File,
    /// One `cmds:` entry of a task.
    TaskCmd,
    /// One `status:` entry of a task.
    TaskStatus,
    /// One `preconditions:` entry of a task.
    TaskPrecondition,
}

/// One unit of shell, as scanned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellSource {
    /// Stable across scans: `file:<path>` for a script, and
    /// `<kind>:<taskfile path>:<task>:<index>` for Taskfile shell.
    pub id: String,
    pub kind: ShellSourceKind,
    /// Repository-relative path of the script, or of the declaring Taskfile.
    pub path: String,
    /// Fully-qualified task name for Taskfile shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Position within the task's `cmds:`/`status:`/`preconditions:` list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// Non-blank, non-comment lines.
    pub lines: u32,
    /// Decision points: `if`/`elif`/`case`/`for`/`while`/`until`, `&&`, `||`.
    pub branches: u32,
    /// Hex SHA-256 of the scanned text, so an unchanged source is recognisable.
    pub digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingLevel {
    Error,
    Warning,
    Info,
    Style,
}

/// One linter finding. Positions are 1-based and relative to the source text
/// (for Taskfile shell: the snippet, not the YAML file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellFinding {
    pub source_id: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub level: FindingLevel,
    /// ShellCheck code without the `SC` prefix (`2086`).
    pub code: u32,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_round_trips() {
        let scan = ShellScan {
            scan_id: Uuid::now_v7(),
            scanned_at: Utc::now(),
            sha: Some("5f17449".into()),
            tool: "shellcheck 0.10.0".into(),
            ci: None,
            sources: vec![ShellSource {
                id: "task_cmd:scripts/tasks/db.yml:db-fresh:0".into(),
                kind: ShellSourceKind::TaskCmd,
                path: "scripts/tasks/db.yml".into(),
                task: Some("db-fresh".into()),
                index: Some(0),
                lines: 12,
                branches: 3,
                digest: "ab".into(),
            }],
            findings: vec![ShellFinding {
                source_id: "task_cmd:scripts/tasks/db.yml:db-fresh:0".into(),
                line: 3,
                column: 5,
                end_line: 3,
                end_column: 9,
                level: FindingLevel::Warning,
                code: 2086,
                message: "Double quote to prevent globbing and word splitting.".into(),
            }],
        };
        let json = serde_json::to_string(&scan).expect("serialize");
        let back: ShellScan = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, scan);
        assert!(json.contains(r#""kind":"task_cmd""#));
        assert!(json.contains(r#""level":"warning""#));
    }
}
