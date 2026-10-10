//! `taskgraph ingest FILE…`: publish events recorded offline (the JSONL a
//! CI job wrote through `$TASKGRAPH_EVENTS_OUT`) to the `TASKGRAPH` stream.
//!
//! Each event keeps its `event_id` and goes out with `Nats-Msg-Id`, so
//! ingesting a file twice within the stream's duplicate window stores it
//! once. Malformed lines are reported with their line number and skipped.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::ExitCode;

use contract_taskgraph::TaskgraphEvent;
use domain_taskgraph::NatsPublisher;
use eyre::{Result, WrapErr};

use crate::context::connect;

#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    published: usize,
    duplicates: usize,
    malformed: usize,
    failed: usize,
}

pub async fn ingest(nats_url: &str, files: &[PathBuf]) -> Result<ExitCode> {
    let js = connect(nats_url).await?;
    let publisher = NatsPublisher::new(js)
        .await
        .wrap_err("creating the TASKGRAPH stream")?;
    let mut ok = true;
    for path in files {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                ok = false;
                continue;
            }
        };
        let mut counts = Counts::default();
        for (n, line) in BufReader::new(file).lines().enumerate() {
            let line_no = n + 1;
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}:{line_no}: unreadable: {e}", path.display());
                    counts.malformed += 1;
                    continue;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let event: TaskgraphEvent = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("{}:{line_no}: malformed event: {e}", path.display());
                    counts.malformed += 1;
                    continue;
                }
            };
            match publisher.send(&event).await {
                Ok(true) => counts.duplicates += 1,
                Ok(false) => counts.published += 1,
                Err(e) => {
                    if counts.failed == 0 {
                        eprintln!("{}:{line_no}: {e}", path.display());
                    }
                    counts.failed += 1;
                }
            }
        }
        ok &= counts.failed == 0;
        println!(
            "{}: {} published, {} duplicates, {} malformed{}",
            path.display(),
            counts.published,
            counts.duplicates,
            counts.malformed,
            if counts.failed > 0 {
                format!(", {} failed", counts.failed)
            } else {
                String::new()
            }
        );
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
