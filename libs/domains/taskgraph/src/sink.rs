//! Non-NATS event sinks: a JSONL file (CI artifacts, offline runs) and a
//! fan-out that feeds several publishers the same facts in order.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use contract_taskgraph::TaskgraphEvent;

use crate::error::{TaskgraphError, TaskgraphResult};
use crate::nats::EventPublisher;

/// Appends one [`TaskgraphEvent`] per line to a file.
///
/// The file is opened in append mode and each event is written with a single
/// `write` of the whole line, so several processes (parallel `task` calls in
/// one CI job) can share a file without interleaving lines; nothing is
/// buffered in-process, so a crash loses at most the event being written.
pub struct FilePublisher {
    path: PathBuf,
    file: Mutex<File>,
}

impl FilePublisher {
    /// Opens `path` for appending, creating it and its parent directories.
    pub fn open(path: impl Into<PathBuf>) -> TaskgraphResult<Self> {
        let path = path.into();
        let io = |source| TaskgraphError::Io {
            path: path.display().to_string(),
            source,
        };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(io)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait]
impl EventPublisher for FilePublisher {
    async fn publish(&self, event: &TaskgraphEvent) -> TaskgraphResult<()> {
        let mut line = serde_json::to_vec(event).map_err(|e| TaskgraphError::Io {
            path: self.path.display().to_string(),
            source: std::io::Error::other(e),
        })?;
        line.push(b'\n');
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        file.write_all(&line)
            .and_then(|()| file.flush())
            .map_err(|source| TaskgraphError::Io {
                path: self.path.display().to_string(),
                source,
            })
    }
}

/// Publishes every event to each sink in turn. Every sink sees every event
/// even when an earlier one fails; the first failure is returned.
pub struct FanOut {
    sinks: Vec<Arc<dyn EventPublisher>>,
}

impl FanOut {
    pub fn new(sinks: Vec<Arc<dyn EventPublisher>>) -> Self {
        Self { sinks }
    }
}

#[async_trait]
impl EventPublisher for FanOut {
    async fn publish(&self, event: &TaskgraphEvent) -> TaskgraphResult<()> {
        let mut first_error = None;
        for sink in &self.sinks {
            if let Err(e) = sink.publish(event).await {
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use contract_taskgraph::{EventBody, RunOutcome};
    use uuid::Uuid;

    use super::*;

    struct Failing;

    #[async_trait]
    impl EventPublisher for Failing {
        async fn publish(&self, _event: &TaskgraphEvent) -> TaskgraphResult<()> {
            Err(TaskgraphError::Nats("down".into()))
        }
    }

    fn finished() -> TaskgraphEvent {
        TaskgraphEvent::new(
            EventBody::RunFinished {
                run_id: Uuid::now_v7(),
                outcome: RunOutcome::Succeeded,
                exit_code: Some(0),
                duration_ms: 5,
                error: None,
            },
            Utc::now(),
            None,
        )
    }

    #[tokio::test]
    async fn file_sink_appends_across_opens_and_survives_a_failing_sibling() {
        let dir = std::env::temp_dir().join(format!("taskgraph-sink-{}", Uuid::now_v7()));
        let path = dir.join("nested/events.jsonl");
        let (a, b, c) = (finished(), finished(), finished());

        let file: Arc<dyn EventPublisher> = Arc::new(FilePublisher::open(&path).expect("open"));
        let fan = FanOut::new(vec![Arc::new(Failing), file]);
        assert!(fan.publish(&a).await.is_err(), "the failure is reported");
        assert!(fan.publish(&b).await.is_err());
        // A second process appending to the same file.
        FilePublisher::open(&path)
            .expect("reopen")
            .publish(&c)
            .await
            .expect("append");

        let text = std::fs::read_to_string(&path).expect("read");
        let _ = std::fs::remove_dir_all(&dir);
        let back: Vec<TaskgraphEvent> = text
            .lines()
            .map(|l| serde_json::from_str(l).expect("one event per line"))
            .collect();
        assert_eq!(back, vec![a, b, c]);
    }
}
