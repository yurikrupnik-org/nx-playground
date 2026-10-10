//! History-based estimation.
//!
//! go-task reports inclusive durations (a task "starts" before its deps and
//! "finishes" after its last command), so summing recorded durations over a
//! graph would count shared work many times. Estimates are therefore built
//! from **self time** — an execution's duration minus the span its deps ran
//! and the time its `task:` calls took — and recombined along the graph with
//! go-task's own scheduling rules:
//!
//! ```text
//! total(t) = self(t) + max(total(d) for d in deps(t)) + Σ total(c) for c in calls(t)
//! ```
//!
//! `deps` run concurrently (so only the slowest counts); `cmds` calls run in
//! order (so they add). The critical path follows the slowest dep at each level.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use contract_taskgraph::{TaskOutcome, Via};
use serde::Serialize;
use uuid::Uuid;

use crate::graph::GraphIndex;

/// One finished execution of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sample {
    pub run_id: Uuid,
    pub at: DateTime<Utc>,
    pub outcome: TaskOutcome,
    pub duration_ms: u64,
    pub self_ms: u64,
}

/// Aggregates over a task's recorded executions. Percentiles cover the
/// executions that did the task's job — succeeded or up to date — because a
/// failure's duration says nothing about how long the work takes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TaskStats {
    pub runs: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub up_to_date: usize,
    pub cancelled: usize,
    pub p50_ms: Option<u64>,
    pub p90_ms: Option<u64>,
    pub self_p50_ms: Option<u64>,
    pub self_p90_ms: Option<u64>,
    pub max_ms: Option<u64>,
    pub last_outcome: Option<TaskOutcome>,
    pub last_at: Option<DateTime<Utc>>,
    pub last_duration_ms: Option<u64>,
}

impl TaskStats {
    pub fn from_samples<'a>(samples: impl IntoIterator<Item = &'a Sample>) -> Self {
        let mut stats = Self::default();
        let mut durations = Vec::new();
        let mut self_times = Vec::new();
        for sample in samples {
            stats.runs += 1;
            match sample.outcome {
                TaskOutcome::Succeeded => stats.succeeded += 1,
                TaskOutcome::Failed => stats.failed += 1,
                TaskOutcome::UpToDate => stats.up_to_date += 1,
                TaskOutcome::Cancelled => stats.cancelled += 1,
            }
            if matches!(
                sample.outcome,
                TaskOutcome::Succeeded | TaskOutcome::UpToDate
            ) {
                durations.push(sample.duration_ms);
                self_times.push(sample.self_ms);
            }
            if stats.last_at.is_none_or(|last| sample.at >= last) {
                stats.last_at = Some(sample.at);
                stats.last_outcome = Some(sample.outcome);
                stats.last_duration_ms = Some(sample.duration_ms);
            }
        }
        durations.sort_unstable();
        self_times.sort_unstable();
        stats.p50_ms = percentile(&durations, 50);
        stats.p90_ms = percentile(&durations, 90);
        stats.self_p50_ms = percentile(&self_times, 50);
        stats.self_p90_ms = percentile(&self_times, 90);
        stats.max_ms = durations.last().copied();
        stats
    }
}

/// Nearest-rank percentile of an ascending slice.
pub fn percentile(sorted: &[u64], pct: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (pct * sorted.len()).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

/// Expected duration of running a task, recombined from per-task self time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Estimate {
    pub target: String,
    /// From median self times.
    pub expected_ms: u64,
    /// From p90 self times (pessimistic: every task at its p90 at once).
    pub pessimistic_ms: u64,
    /// Target first, then the slowest dep at each level.
    pub critical_path: Vec<String>,
    /// Tasks the target can run, including itself.
    pub tasks: usize,
    /// Tasks with no usable history, counted as zero.
    pub unknown: Vec<String>,
}

#[derive(Clone, Copy, Default)]
struct Cost {
    expected: u64,
    pessimistic: u64,
}

/// Estimate `target` given each task's `(self_p50, self_p90)`; `None` when
/// the target is not in the graph. Cycles contribute nothing past the back edge.
pub fn estimate<F>(index: &GraphIndex<'_>, target: &str, self_time: F) -> Option<Estimate>
where
    F: Fn(&str) -> Option<(u64, u64)>,
{
    let root = index.resolve(target)?;
    let mut walk = Walk {
        index,
        self_time: &self_time,
        memo: HashMap::new(),
        slowest_dep: HashMap::new(),
        visiting: HashSet::new(),
        unknown: Vec::new(),
    };
    let total = walk.total(root);

    let mut critical_path = vec![root.to_string()];
    let mut cursor = root;
    while let Some(next) = walk.slowest_dep.get(cursor).copied() {
        if critical_path.iter().any(|t| t == next) {
            break;
        }
        critical_path.push(next.to_string());
        cursor = next;
    }
    let mut unknown = walk.unknown;
    unknown.sort_unstable();
    Some(Estimate {
        target: root.to_string(),
        expected_ms: total.expected,
        pessimistic_ms: total.pessimistic,
        critical_path,
        tasks: index.closure(root).len() + 1,
        unknown,
    })
}

struct Walk<'a, 'g, F> {
    index: &'a GraphIndex<'g>,
    self_time: &'a F,
    memo: HashMap<&'g str, Cost>,
    slowest_dep: HashMap<&'g str, &'g str>,
    visiting: HashSet<&'g str>,
    unknown: Vec<String>,
}

impl<'g, F> Walk<'_, 'g, F>
where
    F: Fn(&str) -> Option<(u64, u64)>,
{
    fn total(&mut self, task: &'g str) -> Cost {
        if let Some(cost) = self.memo.get(task) {
            return *cost;
        }
        if !self.visiting.insert(task) {
            return Cost::default();
        }
        let (own_expected, own_pessimistic) = (self.self_time)(task).unwrap_or_else(|| {
            self.unknown.push(task.to_string());
            (0, 0)
        });
        let mut deps = Cost::default();
        let mut calls = Cost::default();
        let mut slowest: Option<(&'g str, u64)> = None;
        for edge in self.index.edges(task) {
            let cost = self.total(edge.task);
            match edge.via {
                Via::Dep => {
                    deps.pessimistic = deps.pessimistic.max(cost.pessimistic);
                    if slowest.is_none_or(|(_, best)| cost.expected > best) {
                        slowest = Some((edge.task, cost.expected));
                    }
                    deps.expected = deps.expected.max(cost.expected);
                }
                Via::Call | Via::Root => {
                    calls.expected += cost.expected;
                    calls.pessimistic += cost.pessimistic;
                }
            }
        }
        self.visiting.remove(task);
        if let Some((dep, _)) = slowest {
            self.slowest_dep.insert(task, dep);
        }
        let cost = Cost {
            expected: own_expected + deps.expected + calls.expected,
            pessimistic: own_pessimistic + deps.pessimistic + calls.pessimistic,
        };
        self.memo.insert(task, cost);
        cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{graph, node};

    #[test]
    fn percentile_is_nearest_rank() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[7], 90), Some(7));
        assert_eq!(percentile(&[1, 2, 3, 4], 50), Some(2));
        assert_eq!(percentile(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 90), Some(9));
    }

    #[test]
    fn stats_ignore_failures_for_percentiles_and_track_the_latest_outcome() {
        let at = |s: i64| DateTime::from_timestamp(s, 0).expect("ts");
        let sample = |s, outcome, ms| Sample {
            run_id: Uuid::nil(),
            at: at(s),
            outcome,
            duration_ms: ms,
            self_ms: ms / 2,
        };
        let stats = TaskStats::from_samples(&[
            sample(1, TaskOutcome::Succeeded, 100),
            sample(3, TaskOutcome::Failed, 9_000),
            sample(2, TaskOutcome::UpToDate, 10),
        ]);
        assert_eq!((stats.runs, stats.failed, stats.up_to_date), (3, 1, 1));
        assert_eq!(stats.p50_ms, Some(10));
        assert_eq!(stats.max_ms, Some(100));
        assert_eq!(stats.self_p90_ms, Some(50));
        assert_eq!(stats.last_outcome, Some(TaskOutcome::Failed));
    }

    /// deps overlap (max), calls add (sum), shared deps are not double counted
    /// along one path, and unknown tasks are reported rather than guessed.
    #[test]
    fn estimate_follows_go_task_scheduling() {
        let g = graph(vec![
            node("ci", &["lint", "test"], &["publish"]),
            node("lint", &["build"], &[]),
            node("test", &["build"], &[]),
            node("build", &[], &[]),
            node("publish", &[], &[]),
        ]);
        let index = GraphIndex::new(&g);
        let times = HashMap::from([
            ("ci", (5, 10)),
            ("lint", (30, 40)),
            ("test", (100, 300)),
            ("build", (60, 90)),
        ]);
        let est = estimate(&index, "ci", |t| times.get(t).copied()).expect("estimate");

        // ci 5 + max(lint 30+60, test 100+60) + publish 0
        assert_eq!(est.expected_ms, 165);
        assert_eq!(est.pessimistic_ms, 10 + (300 + 90));
        assert_eq!(est.critical_path, ["ci", "test", "build"]);
        assert_eq!(est.tasks, 5);
        assert_eq!(est.unknown, ["publish"]);
    }
}
