//! The read model: a pure fold over [`TaskgraphEvent`]s.
//!
//! Everything the drill-down shows is derived here from facts alone, so any
//! reader can rebuild it by replaying the `TASKGRAPH` stream from the first
//! retained message — the API does this on start, the CLI for `runs` and
//! `estimate`. Nothing is written back; there is no second source of truth.
//!
//! Memory is bounded by [`Limits`]: the oldest runs and samples fall off, the
//! same way the stream's own retention drops the oldest messages.

use std::collections::{BTreeMap, HashMap, VecDeque};

use chrono::{DateTime, Utc};
use contract_taskgraph::{
    EventBody, Graph, RunOrigin, RunOutcome, TaskNode, TaskOutcome, TaskgraphEvent, TraceRef, Via,
};
use serde::Serialize;
use uuid::Uuid;

use crate::estimate::{Estimate, Sample, TaskStats, estimate};
use crate::graph::{GraphIndex, TreeNode};
use crate::observe::millis_between;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_runs: usize,
    /// Per task, per graph.
    pub max_samples: usize,
    /// Per execution.
    pub max_commands: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_runs: 1_000,
            max_samples: 200,
            max_commands: 200,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEntry {
    pub graph: Graph,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandRecord {
    pub at: DateTime<Utc>,
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Execution {
    pub instance: u32,
    pub task: String,
    pub parent: Option<u32>,
    pub via: Via,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: Option<TaskOutcome>,
    pub duration_ms: Option<u64>,
    /// Duration minus the deps' span and the calls' durations.
    pub self_ms: Option<u64>,
    pub error: Option<String>,
    pub commands: Vec<CommandRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub run_id: Uuid,
    pub graph_id: String,
    pub target: String,
    pub args: Vec<String>,
    pub host: String,
    pub user: String,
    pub cwd: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: Option<RunOutcome>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub estimate_ms: Option<u64>,
    pub error: Option<String>,
    pub trace: Option<TraceRef>,
    /// CI job / invoker / commit the run started from.
    pub origin: RunOrigin,
    /// In start order.
    pub executions: Vec<Execution>,
}

impl Run {
    fn execution_mut(&mut self, instance: u32) -> Option<&mut Execution> {
        self.executions.iter_mut().find(|e| e.instance == instance)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphSummary {
    pub id: String,
    pub taskfile: String,
    pub host: String,
    pub digest: String,
    pub published_at: DateTime<Utc>,
    pub tasks: usize,
    pub warnings: usize,
    pub runs: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphView {
    pub graph: Graph,
    pub published_at: DateTime<Utc>,
    /// Public tasks nothing references: where a drill-down starts.
    pub roots: Vec<String>,
    /// Only tasks with history appear.
    pub stats: BTreeMap<String, TaskStats>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependentView {
    pub task: String,
    pub via: Via,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub graph_id: String,
    pub task: TaskNode,
    pub tree: TreeNode,
    pub dependents: Vec<DependentView>,
    pub transitive_dependents: Vec<String>,
    /// Everything this task can run, dependency order.
    pub closure: Vec<String>,
    pub stats: TaskStats,
    pub estimate: Option<Estimate>,
    /// Newest first.
    pub recent: Vec<Sample>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub run_id: Uuid,
    pub graph_id: String,
    pub target: String,
    pub host: String,
    pub user: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: Option<RunOutcome>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub estimate_ms: Option<u64>,
    pub executions: usize,
    pub failed: usize,
    pub trace_id: Option<String>,
    pub origin: RunOrigin,
}

impl From<&Run> for RunSummary {
    fn from(run: &Run) -> Self {
        Self {
            run_id: run.run_id,
            graph_id: run.graph_id.clone(),
            target: run.target.clone(),
            host: run.host.clone(),
            user: run.user.clone(),
            started_at: run.started_at,
            finished_at: run.finished_at,
            outcome: run.outcome,
            exit_code: run.exit_code,
            duration_ms: run.duration_ms,
            estimate_ms: run.estimate_ms,
            executions: run.executions.len(),
            failed: run
                .executions
                .iter()
                .filter(|e| e.outcome == Some(TaskOutcome::Failed))
                .count(),
            trace_id: run.trace.as_ref().map(|t| t.trace_id.clone()),
            origin: run.origin.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Projection {
    limits: Limits,
    graphs: HashMap<String, GraphEntry>,
    runs: HashMap<Uuid, Run>,
    /// Oldest first.
    run_order: VecDeque<Uuid>,
    /// graph id → task → samples, oldest first.
    samples: HashMap<String, HashMap<String, VecDeque<Sample>>>,
    applied: u64,
}

impl Projection {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// Events that changed state so far.
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Fold one fact in. Returns `false` when it changed nothing: a duplicate
    /// (at-least-once delivery) or a fact about a run whose start is no longer
    /// retained.
    pub fn apply(&mut self, event: &TaskgraphEvent) -> bool {
        let changed = match &event.body {
            EventBody::GraphPublished { graph } => {
                self.graphs.insert(
                    graph.id.clone(),
                    GraphEntry {
                        graph: graph.clone(),
                        published_at: event.at,
                    },
                );
                true
            }
            EventBody::RunStarted {
                run_id,
                graph_id,
                target,
                args,
                host,
                user,
                cwd,
                estimate_ms,
                origin,
            } => {
                if self.runs.contains_key(run_id) {
                    return false;
                }
                self.runs.insert(
                    *run_id,
                    Run {
                        run_id: *run_id,
                        graph_id: graph_id.clone(),
                        target: target.clone(),
                        args: args.clone(),
                        host: host.clone(),
                        user: user.clone(),
                        cwd: cwd.clone(),
                        started_at: event.at,
                        finished_at: None,
                        outcome: None,
                        exit_code: None,
                        duration_ms: None,
                        estimate_ms: *estimate_ms,
                        error: None,
                        trace: event.trace.clone(),
                        origin: origin.clone(),
                        executions: Vec::new(),
                    },
                );
                self.run_order.push_back(*run_id);
                while self.run_order.len() > self.limits.max_runs {
                    if let Some(old) = self.run_order.pop_front() {
                        self.runs.remove(&old);
                    }
                }
                true
            }
            EventBody::TaskStarted {
                run_id,
                instance,
                task,
                parent,
                via,
            } => match self.runs.get_mut(run_id) {
                Some(run) if !run.executions.iter().any(|e| e.instance == *instance) => {
                    run.executions.push(Execution {
                        instance: *instance,
                        task: task.clone(),
                        parent: *parent,
                        via: *via,
                        started_at: event.at,
                        finished_at: None,
                        outcome: None,
                        duration_ms: None,
                        self_ms: None,
                        error: None,
                        commands: Vec::new(),
                    });
                    true
                }
                _ => false,
            },
            EventBody::CommandStarted {
                run_id,
                instance,
                command,
                ..
            } => {
                let max = self.limits.max_commands;
                match self
                    .runs
                    .get_mut(run_id)
                    .and_then(|r| r.execution_mut(*instance))
                {
                    Some(execution) if execution.commands.len() < max => {
                        execution.commands.push(CommandRecord {
                            at: event.at,
                            command: command.clone(),
                        });
                        true
                    }
                    _ => false,
                }
            }
            EventBody::TaskFinished {
                run_id,
                instance,
                task,
                outcome,
                duration_ms,
                error,
            } => self.finish_task(
                *run_id,
                *instance,
                task,
                *outcome,
                *duration_ms,
                error.clone(),
                event.at,
            ),
            EventBody::RunFinished {
                run_id,
                outcome,
                exit_code,
                duration_ms,
                error,
            } => match self.runs.get_mut(run_id) {
                Some(run) if run.finished_at.is_none() => {
                    run.finished_at = Some(event.at);
                    run.outcome = Some(*outcome);
                    run.exit_code = *exit_code;
                    run.duration_ms = Some(*duration_ms);
                    run.error = error.clone();
                    true
                }
                _ => false,
            },
        };
        if changed {
            self.applied += 1;
        }
        changed
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_task(
        &mut self,
        run_id: Uuid,
        instance: u32,
        task: &str,
        outcome: TaskOutcome,
        duration_ms: u64,
        error: Option<String>,
        at: DateTime<Utc>,
    ) -> bool {
        let Some(run) = self.runs.get_mut(&run_id) else {
            return false;
        };
        let Some((started_at, first_command)) = run
            .executions
            .iter()
            .find(|e| e.instance == instance && e.finished_at.is_none())
            .map(|e| (e.started_at, e.commands.first().map(|c| c.at)))
        else {
            return false;
        };
        // Children are finished before their parent: deps before it runs its
        // commands, calls while it does.
        let mut deps_end: Option<DateTime<Utc>> = None;
        let mut calls_ms = 0u64;
        for child in run.executions.iter().filter(|e| e.parent == Some(instance)) {
            match child.via {
                Via::Dep => {
                    if let Some(end) = child.finished_at {
                        deps_end = Some(deps_end.map_or(end, |d| d.max(end)));
                    }
                }
                Via::Call => calls_ms += child.duration_ms.unwrap_or(0),
                Via::Root => {}
            }
        }
        // A dep go-task did not start again for this execution (`run: once`,
        // or deduplicated while already running) was still waited for: its
        // execution elsewhere in the run bounds this task's deps phase, which
        // ends at the first command this task echoed (or its finish).
        let deps_phase_end = first_command.unwrap_or(at);
        if let Some(node) = self
            .graphs
            .get(&run.graph_id)
            .and_then(|entry| entry.graph.tasks.iter().find(|t| t.name == task))
        {
            for dep in node.deps.iter().filter(|d| d.resolved) {
                let own_child = run
                    .executions
                    .iter()
                    .any(|e| e.parent == Some(instance) && e.task == dep.name);
                if own_child {
                    continue;
                }
                let shared_end = run
                    .executions
                    .iter()
                    .filter(|e| e.task == dep.name)
                    .filter_map(|e| e.finished_at)
                    .filter(|end| *end <= deps_phase_end)
                    .max();
                if let Some(end) = shared_end {
                    deps_end = Some(deps_end.map_or(end, |d| d.max(end)));
                }
            }
        }
        let deps_ms = deps_end.map_or(0, |end| millis_between(started_at, end));
        let self_ms = duration_ms.saturating_sub(deps_ms + calls_ms);

        let graph_id = run.graph_id.clone();
        if let Some(execution) = run.execution_mut(instance) {
            execution.finished_at = Some(at);
            execution.outcome = Some(outcome);
            execution.duration_ms = Some(duration_ms);
            execution.self_ms = Some(self_ms);
            execution.error = error;
        }

        let samples = self
            .samples
            .entry(graph_id)
            .or_default()
            .entry(task.to_string())
            .or_default();
        samples.push_back(Sample {
            run_id,
            at,
            outcome,
            duration_ms,
            self_ms,
        });
        while samples.len() > self.limits.max_samples {
            samples.pop_front();
        }
        true
    }

    fn task_samples(&self, graph_id: &str, task: &str) -> Option<&VecDeque<Sample>> {
        self.samples.get(graph_id).and_then(|tasks| tasks.get(task))
    }

    pub fn stats(&self, graph_id: &str, task: &str) -> TaskStats {
        self.task_samples(graph_id, task)
            .map(TaskStats::from_samples)
            .unwrap_or_default()
    }

    pub fn graph_entry(&self, graph_id: &str) -> Option<&GraphEntry> {
        self.graphs.get(graph_id)
    }

    pub fn estimate(&self, graph_id: &str, task: &str) -> Option<Estimate> {
        let entry = self.graphs.get(graph_id)?;
        let index = GraphIndex::new(&entry.graph);
        estimate(&index, task, |t| {
            let stats = self.stats(graph_id, t);
            Some((stats.self_p50_ms?, stats.self_p90_ms?))
        })
    }

    /// Newest publication first.
    pub fn graphs(&self) -> Vec<GraphSummary> {
        let mut out: Vec<GraphSummary> = self
            .graphs
            .values()
            .map(|entry| GraphSummary {
                id: entry.graph.id.clone(),
                taskfile: entry.graph.taskfile.clone(),
                host: entry.graph.host.clone(),
                digest: entry.graph.digest.clone(),
                published_at: entry.published_at,
                tasks: entry.graph.tasks.len(),
                warnings: entry.graph.warnings.len(),
                runs: self
                    .runs
                    .values()
                    .filter(|r| r.graph_id == entry.graph.id)
                    .count(),
            })
            .collect();
        out.sort_by_key(|g| std::cmp::Reverse(g.published_at));
        out
    }

    pub fn graph(&self, graph_id: &str) -> Option<GraphView> {
        let entry = self.graphs.get(graph_id)?;
        let index = GraphIndex::new(&entry.graph);
        let stats = self
            .samples
            .get(graph_id)
            .map(|tasks| {
                tasks
                    .iter()
                    .map(|(task, samples)| (task.clone(), TaskStats::from_samples(samples)))
                    .collect()
            })
            .unwrap_or_default();
        Some(GraphView {
            graph: entry.graph.clone(),
            published_at: entry.published_at,
            roots: index.roots().into_iter().map(str::to_string).collect(),
            stats,
        })
    }

    /// Drill-down for one task (name or alias), tree cut at `depth` levels.
    pub fn task(&self, graph_id: &str, name: &str, depth: usize) -> Option<TaskView> {
        let entry = self.graphs.get(graph_id)?;
        let index = GraphIndex::new(&entry.graph);
        let node = index.task(name)?;
        let canonical = node.name.as_str();
        let recent = self
            .task_samples(graph_id, canonical)
            .map(|s| s.iter().rev().take(50).cloned().collect())
            .unwrap_or_default();
        Some(TaskView {
            graph_id: graph_id.to_string(),
            task: node.clone(),
            tree: index.tree(canonical, depth)?,
            dependents: index
                .dependents(canonical)
                .iter()
                .map(|e| DependentView {
                    task: e.task.to_string(),
                    via: e.via,
                })
                .collect(),
            transitive_dependents: index
                .transitive_dependents(canonical)
                .into_iter()
                .map(str::to_string)
                .collect(),
            closure: index
                .closure(canonical)
                .into_iter()
                .map(str::to_string)
                .collect(),
            stats: self.stats(graph_id, canonical),
            estimate: self.estimate(graph_id, canonical),
            recent,
        })
    }

    /// Newest first, optionally only one graph's runs.
    pub fn runs(&self, graph_id: Option<&str>, limit: usize) -> Vec<RunSummary> {
        self.run_order
            .iter()
            .rev()
            .filter_map(|id| self.runs.get(id))
            .filter(|run| graph_id.is_none_or(|g| run.graph_id == g))
            .take(limit)
            .map(RunSummary::from)
            .collect()
    }

    pub fn run(&self, run_id: Uuid) -> Option<&Run> {
        self.runs.get(&run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{graph, node};

    struct Clock(DateTime<Utc>);

    impl Clock {
        fn at(&self, ms: i64) -> DateTime<Utc> {
            self.0 + chrono::Duration::milliseconds(ms)
        }
    }

    fn ev(body: EventBody, at: DateTime<Utc>) -> TaskgraphEvent {
        TaskgraphEvent::new(body, at, None)
    }

    /// ci: deps [lint] (0–400ms), then calls publish (500–800ms), ends 1000ms.
    /// Self time = 1000 − 400 (deps span) − 300 (call) = 300.
    #[test]
    fn self_time_subtracts_the_deps_span_and_the_calls() {
        let clock = Clock(DateTime::from_timestamp(10_000, 0).expect("ts"));
        let g = graph(vec![
            node("ci", &["lint"], &["publish"]),
            node("lint", &[], &[]),
            node("publish", &[], &[]),
        ]);
        let run_id = Uuid::now_v7();
        let mut p = Projection::new(Limits::default());
        let started = |instance, task: &str, parent, via| EventBody::TaskStarted {
            run_id,
            instance,
            task: task.into(),
            parent,
            via,
        };
        let finished = |instance, task: &str, ms| EventBody::TaskFinished {
            run_id,
            instance,
            task: task.into(),
            outcome: TaskOutcome::Succeeded,
            duration_ms: ms,
            error: None,
        };
        let events = [
            ev(EventBody::GraphPublished { graph: g }, clock.at(0)),
            ev(
                EventBody::RunStarted {
                    run_id,
                    graph_id: "g".into(),
                    target: "ci".into(),
                    args: vec![],
                    host: "h".into(),
                    user: "u".into(),
                    cwd: "/".into(),
                    estimate_ms: None,
                    origin: RunOrigin::default(),
                },
                clock.at(0),
            ),
            ev(started(1, "ci", None, Via::Root), clock.at(0)),
            ev(started(2, "lint", Some(1), Via::Dep), clock.at(0)),
            ev(finished(2, "lint", 400), clock.at(400)),
            ev(started(3, "publish", Some(1), Via::Call), clock.at(500)),
            ev(finished(3, "publish", 300), clock.at(800)),
            ev(finished(1, "ci", 1000), clock.at(1000)),
            ev(
                EventBody::RunFinished {
                    run_id,
                    outcome: RunOutcome::Succeeded,
                    exit_code: Some(0),
                    duration_ms: 1000,
                    error: None,
                },
                clock.at(1000),
            ),
        ];
        for e in &events {
            assert!(p.apply(e));
        }
        assert!(!p.apply(&events[7]), "a redelivered finish changes nothing");

        let run = p.run(run_id).expect("run");
        let ci = run.executions.iter().find(|e| e.task == "ci").expect("ci");
        assert_eq!(ci.self_ms, Some(300));
        assert_eq!(p.stats("g", "ci").self_p50_ms, Some(300));
        assert_eq!(p.stats("g", "ci").p50_ms, Some(1000));

        // Recombined from self times: 300 + lint 400 + publish 300.
        let est = p.estimate("g", "ci").expect("estimate");
        assert_eq!(est.expected_ms, 1000);
        assert!(est.unknown.is_empty());

        let view = p.task("g", "lint", 5).expect("view");
        assert_eq!(view.dependents.len(), 1);
        assert_eq!(view.recent.len(), 1);
        let summary = &p.runs(None, 10)[0];
        assert_eq!((summary.executions, summary.failed), (3, 0));
        assert_eq!(summary.outcome, Some(RunOutcome::Succeeded));
    }

    /// `lint` and `test` both dep on `build` (`run: once`): go-task starts it
    /// once, under `lint`. `test` still waited 400ms for it, so that wait is
    /// not `test`'s own work even though `test` has no child execution.
    #[test]
    fn a_shared_run_once_dep_still_bounds_the_waiting_tasks_self_time() {
        let clock = Clock(DateTime::from_timestamp(10_000, 0).expect("ts"));
        let g = graph(vec![
            node("ci", &["lint", "test"], &[]),
            node("lint", &["build"], &[]),
            node("test", &["build"], &[]),
            node("build", &[], &[]),
        ]);
        let run_id = Uuid::now_v7();
        let started = |instance, task: &str, parent| EventBody::TaskStarted {
            run_id,
            instance,
            task: task.into(),
            parent,
            via: if parent.is_some() {
                Via::Dep
            } else {
                Via::Root
            },
        };
        let finished = |instance, task: &str, ms| EventBody::TaskFinished {
            run_id,
            instance,
            task: task.into(),
            outcome: TaskOutcome::Succeeded,
            duration_ms: ms,
            error: None,
        };
        let mut p = Projection::new(Limits::default());
        for e in [
            ev(EventBody::GraphPublished { graph: g }, clock.at(0)),
            ev(
                EventBody::RunStarted {
                    run_id,
                    graph_id: "g".into(),
                    target: "ci".into(),
                    args: vec![],
                    host: "h".into(),
                    user: "u".into(),
                    cwd: "/".into(),
                    estimate_ms: None,
                    origin: RunOrigin::default(),
                },
                clock.at(0),
            ),
            ev(started(1, "ci", None), clock.at(0)),
            ev(started(2, "test", Some(1)), clock.at(0)),
            ev(started(3, "lint", Some(1)), clock.at(0)),
            ev(started(4, "build", Some(3)), clock.at(0)),
            ev(finished(4, "build", 400), clock.at(400)),
            ev(
                EventBody::CommandStarted {
                    run_id,
                    instance: 2,
                    task: "test".into(),
                    command: "go test".into(),
                },
                clock.at(400),
            ),
            ev(finished(2, "test", 1000), clock.at(1000)),
        ] {
            p.apply(&e);
        }
        assert_eq!(p.stats("g", "test").self_p50_ms, Some(600));
    }

    #[test]
    fn facts_about_an_unknown_run_are_ignored_and_old_runs_are_evicted() {
        let mut p = Projection::new(Limits {
            max_runs: 2,
            ..Limits::default()
        });
        let now = Utc::now();
        assert!(!p.apply(&ev(
            EventBody::TaskStarted {
                run_id: Uuid::now_v7(),
                instance: 1,
                task: "a".into(),
                parent: None,
                via: Via::Root,
            },
            now
        )));
        let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
        for id in &ids {
            p.apply(&ev(
                EventBody::RunStarted {
                    run_id: *id,
                    graph_id: "g".into(),
                    target: "a".into(),
                    args: vec![],
                    host: "h".into(),
                    user: "u".into(),
                    cwd: "/".into(),
                    estimate_ms: None,
                    origin: RunOrigin::default(),
                },
                now,
            ));
        }
        let kept: Vec<Uuid> = p.runs(None, 10).iter().map(|r| r.run_id).collect();
        assert_eq!(kept, [ids[2], ids[1]]);
        assert!(p.run(ids[0]).is_none());
    }
}
