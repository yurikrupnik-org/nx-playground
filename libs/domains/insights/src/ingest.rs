//! CI artifacts → the event log and the store.
//!
//! GitHub Actions jobs upload `taskgraph-events-<job>-<attempt>` (one or more
//! `*.jsonl` files of `TaskgraphEvent`s) and `taskgraph-shell-scan-<attempt>`
//! (`shell-scan.json`, a `ShellScan`). Events are re-published to `TASKGRAPH`
//! (the event log stays the one source the warehouse reads); shell scans go
//! straight to Postgres. Each consumed artifact is ledgered by id, so it is
//! downloaded once; a re-publish that slips past the stream's duplicate
//! window is absorbed by the warehouse's `event_id` ledger.

use std::io::{Cursor, Read};

use contract_taskgraph::TaskgraphEvent;
use contract_taskgraph::shell::ShellScan;
use domain_taskgraph::EventPublisher;
use tracing::{debug, warn};

use crate::error::{InsightsError, InsightsResult};
use crate::github::{Artifact, GitHub};
use crate::store::{ArtifactLedger, Store};

pub const EVENTS_PREFIX: &str = "taskgraph-events-";
pub const SHELL_SCAN_PREFIX: &str = "taskgraph-shell-scan";
/// A single archive member larger than this is refused (corrupt or hostile).
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Events,
    ShellScan,
}

impl ArtifactKind {
    /// Classify by name; `None` for artifacts this service does not read.
    pub fn of(name: &str) -> Option<Self> {
        if name.starts_with(EVENTS_PREFIX) {
            Some(Self::Events)
        } else if name.starts_with(SHELL_SCAN_PREFIX) {
            Some(Self::ShellScan)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::ShellScan => "shell_scan",
        }
    }
}

/// One file of an artifact archive.
#[derive(Debug, Clone)]
pub struct ZipEntry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Every file of a zip archive, in archive order.
pub fn unzip(bytes: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a zip archive: {e}"))?;
    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("zip entry {i}: {e}"))?;
        if file.is_dir() {
            continue;
        }
        if file.size() > MAX_ENTRY_BYTES {
            return Err(format!(
                "zip entry {} is {} bytes (limit {MAX_ENTRY_BYTES})",
                file.name(),
                file.size()
            ));
        }
        let mut data = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
        file.read_to_end(&mut data)
            .map_err(|e| format!("zip entry {}: {e}", file.name()))?;
        entries.push(ZipEntry {
            name: file.name().to_string(),
            data,
        });
    }
    Ok(entries)
}

#[derive(Debug, Default)]
pub struct DecodedEvents {
    pub events: Vec<TaskgraphEvent>,
    /// Non-blank lines that are not a `TaskgraphEvent`.
    pub malformed: usize,
}

/// Events of every `*.jsonl` member, one per non-blank line, in file order.
pub fn decode_events(entries: &[ZipEntry]) -> DecodedEvents {
    let mut decoded = DecodedEvents::default();
    for entry in entries.iter().filter(|e| e.name.ends_with(".jsonl")) {
        for line in String::from_utf8_lossy(&entry.data).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<TaskgraphEvent>(line) {
                Ok(event) => decoded.events.push(event),
                Err(_) => decoded.malformed += 1,
            }
        }
    }
    decoded
}

/// Every `*.json` member as a `ShellScan`; one bad member fails the artifact.
pub fn decode_shell_scans(entries: &[ZipEntry]) -> Result<Vec<ShellScan>, String> {
    entries
        .iter()
        .filter(|e| e.name.ends_with(".json"))
        .map(|e| serde_json::from_slice(&e.data).map_err(|err| format!("{}: {err}", e.name)))
        .collect()
}

/// Consumes one run's artifacts.
pub struct ArtifactIngest<'a> {
    pub github: &'a GitHub,
    pub store: &'a Store,
    pub publisher: &'a dyn EventPublisher,
}

impl ArtifactIngest<'_> {
    /// Ingest every not-yet-ledgered artifact of `run_id`. Returns the number
    /// of items (events published + scans stored). A transient failure
    /// (GitHub, NATS, Postgres) aborts before ledgering, so the artifact is
    /// retried next cycle.
    pub async fn ingest_run(&self, run_id: i64) -> InsightsResult<u64> {
        let mut items = 0;
        for artifact in self.github.run_artifacts(run_id).await? {
            let Some(kind) = ArtifactKind::of(&artifact.name) else {
                continue;
            };
            if self.store.is_artifact_ingested(artifact.id).await? {
                continue;
            }
            items += self.ingest_artifact(run_id, kind, &artifact).await?;
        }
        Ok(items)
    }

    async fn ingest_artifact(
        &self,
        run_id: i64,
        kind: ArtifactKind,
        artifact: &Artifact,
    ) -> InsightsResult<u64> {
        let ledger = |items: usize, malformed: usize, error: Option<&'static str>| ArtifactLedger {
            artifact_id: artifact.id,
            run_id,
            name: &artifact.name,
            kind: kind.as_str(),
            items: i32::try_from(items).unwrap_or(i32::MAX),
            malformed: i32::try_from(malformed).unwrap_or(i32::MAX),
            error,
        };
        if artifact.expired {
            self.store
                .ledger_artifact(&ledger(0, 0, Some("expired")))
                .await?;
            return Ok(0);
        }
        let bytes = match self.github.download_artifact(artifact.id).await {
            Ok(bytes) => bytes,
            Err(e) if e.is_gone() => {
                self.store
                    .ledger_artifact(&ledger(0, 0, Some("gone")))
                    .await?;
                return Ok(0);
            }
            Err(e) => return Err(e.into()),
        };
        let entries = match unzip(&bytes) {
            Ok(entries) => entries,
            Err(e) => {
                self.store
                    .record_sync_error("artifacts", Some(&artifact.name), &e)
                    .await?;
                self.store
                    .ledger_artifact(&ledger(0, 0, Some("corrupt archive")))
                    .await?;
                return Ok(0);
            }
        };
        match kind {
            ArtifactKind::Events => {
                let decoded = decode_events(&entries);
                for event in &decoded.events {
                    self.publisher
                        .publish(event)
                        .await
                        .map_err(|e| InsightsError::Nats(e.to_string()))?;
                }
                if decoded.malformed > 0 {
                    warn!(artifact = %artifact.name, malformed = decoded.malformed, "skipped malformed event lines");
                    self.store
                        .record_sync_error(
                            "artifacts",
                            Some(&artifact.name),
                            &format!("{} malformed event lines skipped", decoded.malformed),
                        )
                        .await?;
                }
                debug!(artifact = %artifact.name, events = decoded.events.len(), "published artifact events");
                self.store
                    .ledger_artifact(&ledger(decoded.events.len(), decoded.malformed, None))
                    .await?;
                Ok(decoded.events.len() as u64)
            }
            ArtifactKind::ShellScan => match decode_shell_scans(&entries) {
                Ok(scans) => {
                    for scan in &scans {
                        self.store.store_shell_scan(scan, Some(artifact.id)).await?;
                    }
                    self.store
                        .ledger_artifact(&ledger(scans.len(), 0, None))
                        .await?;
                    Ok(scans.len() as u64)
                }
                Err(e) => {
                    self.store
                        .record_sync_error("artifacts", Some(&artifact.name), &e)
                        .await?;
                    self.store
                        .ledger_artifact(&ledger(0, 1, Some("undecodable shell scan")))
                        .await?;
                    Ok(0)
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use chrono::Utc;
    use contract_taskgraph::{EventBody, RunOutcome};
    use uuid::Uuid;
    use zip::write::SimpleFileOptions;

    use super::*;

    /// An archive shaped like the one GitHub serves for an artifact (deflate).
    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in files {
            writer.start_file(*name, options).expect("start file");
            writer.write_all(data).expect("write");
        }
        writer.finish().expect("finish").into_inner()
    }

    fn event(body: EventBody) -> String {
        serde_json::to_string(&TaskgraphEvent::new(body, Utc::now(), None)).expect("serialize")
    }

    #[test]
    fn events_artifact_keeps_order_and_counts_malformed_lines() {
        let run_id = Uuid::now_v7();
        let finished = |exit_code| {
            event(EventBody::RunFinished {
                run_id,
                outcome: RunOutcome::Failed,
                exit_code: Some(exit_code),
                duration_ms: 5,
                error: None,
            })
        };
        let jsonl = format!(
            "{}\n\n{{\"not\":\"an event\"}}\n{}\n",
            finished(1),
            finished(2)
        );
        let bytes = archive(&[
            ("rust.jsonl", jsonl.as_bytes()),
            ("notes.txt", b"ignored: not a .jsonl member"),
        ]);

        let decoded = decode_events(&unzip(&bytes).expect("unzip"));
        assert_eq!(decoded.malformed, 1);
        let exit_codes: Vec<Option<i32>> = decoded
            .events
            .iter()
            .map(|e| match &e.body {
                EventBody::RunFinished { exit_code, .. } => *exit_code,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(exit_codes, vec![Some(1), Some(2)]);
    }

    #[test]
    fn shell_scan_artifact_decodes_and_a_corrupt_one_is_refused() {
        let scan = ShellScan {
            scan_id: Uuid::now_v7(),
            scanned_at: Utc::now(),
            sha: Some("abc".into()),
            tool: "shellcheck 0.10.0".into(),
            ci: None,
            sources: vec![],
            findings: vec![],
        };
        let good = archive(&[("shell-scan.json", &serde_json::to_vec(&scan).expect("json"))]);
        assert_eq!(
            decode_shell_scans(&unzip(&good).expect("unzip")),
            Ok(vec![scan])
        );

        let truncated = archive(&[("shell-scan.json", b"{\"scan_id\":")]);
        assert!(decode_shell_scans(&unzip(&truncated).expect("unzip")).is_err());
        assert!(unzip(b"this is not a zip archive").is_err());
        assert_eq!(
            ArtifactKind::of("taskgraph-events-rust-1"),
            Some(ArtifactKind::Events)
        );
        assert_eq!(
            ArtifactKind::of("taskgraph-shell-scan-1"),
            Some(ArtifactKind::ShellScan)
        );
        assert_eq!(ArtifactKind::of("coverage"), None);
    }
}
