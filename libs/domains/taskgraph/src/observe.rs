//! Turns `task --verbose` stderr into execution facts.
//!
//! go-task is the executor; this module only watches it. With `--verbose`
//! go-task writes one line per lifecycle transition to stderr:
//!
//! ```text
//! task: "build" started
//! task: [build] cargo build
//! task: "build" finished
//! task: Task "fmt" is up to date
//! task: "lint" failed: exit status 1
//! ```
//!
//! Lines carry task *names*, but go-task runs a task once per reference and
//! runs `deps` concurrently, so [`Tracker`] assigns each start a run-scoped
//! instance number and infers which open execution pulled it in from the
//! graph's edges. A finish/command line for a name that has several open
//! executions is attributed to the oldest — the one go-task started first.

use std::borrow::Cow;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use contract_taskgraph::{EventBody, RunOutcome, TaskOutcome, Via};
use uuid::Uuid;

use crate::graph::GraphIndex;

/// One classified stderr line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line<'a> {
    Started(&'a str),
    Finished(&'a str),
    Failed {
        task: &'a str,
        error: &'a str,
    },
    UpToDate(&'a str),
    Command {
        task: &'a str,
        command: &'a str,
    },
    /// go-task's final verdict on a run that failed (`Failed to run task …`,
    /// `Task "x" does not exist`).
    RunFailed(&'a str),
    /// Verbose-only chatter the user did not ask for (hidden unless `--verbose`).
    Chatter,
    /// Anything else: the user's own stderr, or go-task messages shown without `--verbose`.
    Other,
}

impl Line<'_> {
    /// Whether go-task would print this line without `--verbose`.
    pub fn shown_without_verbose(&self) -> bool {
        !matches!(
            self,
            Line::Started(_) | Line::Finished(_) | Line::Failed { .. } | Line::Chatter
        )
    }
}

/// Remove ANSI escape sequences (CSI `ESC [ … letter`), which go-task emits
/// when it believes stderr is a colour terminal.
pub fn strip_ansi(line: &str) -> Cow<'_, str> {
    if !line.contains('\u{1b}') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Classify a line that has already been through [`strip_ansi`].
pub fn classify(line: &str) -> Line<'_> {
    let Some(rest) = line.strip_prefix("task: ") else {
        return Line::Other;
    };
    if let Some(quoted) = rest.strip_prefix('"')
        && let Some((task, tail)) = quoted.split_once('"')
    {
        return match tail {
            " started" => Line::Started(task),
            " finished" => Line::Finished(task),
            _ => match tail.strip_prefix(" failed: ") {
                Some(error) => Line::Failed { task, error },
                None => Line::Other,
            },
        };
    }
    if let Some(bracketed) = rest.strip_prefix('[')
        && let Some((task, command)) = bracketed.split_once("] ")
    {
        return Line::Command { task, command };
    }
    if let Some(named) = rest.strip_prefix("Task \"") {
        if let Some(task) = named.strip_suffix("\" is up to date") {
            return Line::UpToDate(task);
        }
        if named.ends_with("\" does not exist") {
            return Line::RunFailed(rest);
        }
    }
    if rest.starts_with("Failed to run task ") {
        return Line::RunFailed(rest);
    }
    const CHATTER: [&str; 4] = [
        "status command ",
        "dynamic variable:",
        "skipping ",
        "No changes detected",
    ];
    if CHATTER.iter().any(|p| rest.starts_with(p)) {
        return Line::Chatter;
    }
    Line::Other
}

#[derive(Debug, Clone)]
struct Open {
    instance: u32,
    task: String,
    started_at: DateTime<Utc>,
    /// A command was echoed: the deps phase is over, later children are calls.
    in_cmds: bool,
}

/// Per-run state machine: stderr lines in, [`EventBody`] facts out.
pub struct Tracker<'g> {
    run_id: Uuid,
    index: &'g GraphIndex<'g>,
    target: String,
    next_instance: u32,
    /// Open executions in start order.
    open: Vec<Open>,
    /// Every instance's parent and whether it failed (for the final verdict).
    parents: HashMap<u32, Option<u32>>,
    failed: Vec<u32>,
    run_error: Option<String>,
}

impl<'g> Tracker<'g> {
    pub fn new(run_id: Uuid, index: &'g GraphIndex<'g>, target: &str) -> Self {
        Self {
            run_id,
            index,
            target: index.resolve(target).unwrap_or(target).to_string(),
            next_instance: 1,
            open: Vec::new(),
            parents: HashMap::new(),
            failed: Vec::new(),
            run_error: None,
        }
    }

    /// Feed one classified line observed at `at`.
    pub fn observe(&mut self, line: &Line<'_>, at: DateTime<Utc>) -> Vec<EventBody> {
        match *line {
            Line::Started(task) => vec![self.start(task, at)],
            Line::Finished(task) => self.close(task, TaskOutcome::Succeeded, None, at),
            Line::UpToDate(task) => self.close(task, TaskOutcome::UpToDate, None, at),
            Line::Failed { task, error } => {
                self.close(task, TaskOutcome::Failed, Some(error.to_string()), at)
            }
            Line::Command { task, command } => self.command(task, command),
            Line::RunFailed(message) => {
                self.run_error = Some(message.to_string());
                Vec::new()
            }
            Line::Chatter | Line::Other => Vec::new(),
        }
    }

    fn start(&mut self, task: &str, at: DateTime<Utc>) -> EventBody {
        let (parent, via) = self.infer_parent(task);
        let instance = self.next_instance;
        self.next_instance += 1;
        self.parents.insert(instance, parent);
        self.open.push(Open {
            instance,
            task: task.to_string(),
            started_at: at,
            in_cmds: false,
        });
        EventBody::TaskStarted {
            run_id: self.run_id,
            instance,
            task: task.to_string(),
            parent,
            via,
        }
    }

    /// The most recently started open execution with an edge to `task`.
    fn infer_parent(&self, task: &str) -> (Option<u32>, Via) {
        for open in self.open.iter().rev() {
            let edges = self.index.edges(&open.task);
            let dep = edges.iter().any(|e| e.via == Via::Dep && e.task == task);
            let call = edges.iter().any(|e| e.via == Via::Call && e.task == task);
            let via = match (dep, call) {
                (true, true) if open.in_cmds => Via::Call,
                (true, _) => Via::Dep,
                (false, true) => Via::Call,
                (false, false) => continue,
            };
            return (Some(open.instance), via);
        }
        (None, Via::Root)
    }

    fn command(&mut self, task: &str, command: &str) -> Vec<EventBody> {
        let Some(open) = self.open.iter_mut().find(|o| o.task == task) else {
            return Vec::new();
        };
        open.in_cmds = true;
        vec![EventBody::CommandStarted {
            run_id: self.run_id,
            instance: open.instance,
            task: task.to_string(),
            command: command.to_string(),
        }]
    }

    fn close(
        &mut self,
        task: &str,
        outcome: TaskOutcome,
        error: Option<String>,
        at: DateTime<Utc>,
    ) -> Vec<EventBody> {
        let Some(pos) = self.open.iter().position(|o| o.task == task) else {
            return Vec::new();
        };
        let open = self.open.remove(pos);
        // A finished child ends its parent's deps phase only once the parent
        // echoes a command; nothing to update here.
        if outcome == TaskOutcome::Failed {
            self.failed.push(open.instance);
        }
        vec![finished(self.run_id, &open, outcome, error, at)]
    }

    /// go-task exited with `exit_code`: close what is still open and emit the
    /// run verdict. An open execution whose descendant failed failed too; any
    /// other was cut short.
    pub fn finish(
        mut self,
        exit_code: Option<i32>,
        started_at: DateTime<Utc>,
        at: DateTime<Utc>,
    ) -> Vec<EventBody> {
        let mut events = Vec::new();
        let open = std::mem::take(&mut self.open);
        for execution in open.iter().rev() {
            let failed_below = self
                .failed
                .iter()
                .any(|f| self.descends(*f, execution.instance));
            let (outcome, error) = if failed_below {
                (
                    TaskOutcome::Failed,
                    Some("a task it ran failed".to_string()),
                )
            } else {
                (
                    TaskOutcome::Cancelled,
                    Some("still running when go-task exited".to_string()),
                )
            };
            events.push(finished(self.run_id, execution, outcome, error, at));
        }
        let outcome = if exit_code == Some(0) {
            RunOutcome::Succeeded
        } else {
            RunOutcome::Failed
        };
        let error = match (outcome, self.run_error.take()) {
            (RunOutcome::Failed, None) => Some(match exit_code {
                Some(code) => format!("task {} exited with status {code}", self.target),
                None => format!("task {} was killed by a signal", self.target),
            }),
            (_, error) => error,
        };
        events.push(EventBody::RunFinished {
            run_id: self.run_id,
            outcome,
            exit_code,
            duration_ms: millis_between(started_at, at),
            error,
        });
        events
    }

    fn descends(&self, mut instance: u32, ancestor: u32) -> bool {
        while let Some(Some(parent)) = self.parents.get(&instance) {
            if *parent == ancestor {
                return true;
            }
            instance = *parent;
        }
        false
    }
}

fn finished(
    run_id: Uuid,
    open: &Open,
    outcome: TaskOutcome,
    error: Option<String>,
    at: DateTime<Utc>,
) -> EventBody {
    EventBody::TaskFinished {
        run_id,
        instance: open.instance,
        task: open.task.clone(),
        outcome,
        duration_ms: millis_between(open.started_at, at),
        error,
    }
}

pub fn millis_between(from: DateTime<Utc>, to: DateTime<Utc>) -> u64 {
    u64::try_from((to - from).num_milliseconds()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{graph, node};

    #[test]
    fn classifies_go_task_verbose_lines() {
        assert_eq!(
            classify("task: \"rust:lint\" started"),
            Line::Started("rust:lint")
        );
        assert_eq!(classify("task: \"a\" finished"), Line::Finished("a"));
        assert_eq!(
            classify("task: \"d\" failed: exit status 3"),
            Line::Failed {
                task: "d",
                error: "exit status 3"
            }
        );
        assert_eq!(
            classify("task: [b] echo \"x] y\""),
            Line::Command {
                task: "b",
                command: "echo \"x] y\""
            }
        );
        assert_eq!(
            classify("task: Task \"c\" is up to date"),
            Line::UpToDate("c")
        );
        assert!(matches!(
            classify("task: Failed to run task \"d\": exit status 3"),
            Line::RunFailed(_)
        ));
        assert_eq!(
            classify("task: Task \"check\" does not exist"),
            Line::RunFailed("Task \"check\" does not exist")
        );
        assert_eq!(
            classify("task: status command true exited zero"),
            Line::Chatter
        );
        assert_eq!(classify("hello from the user's command"), Line::Other);
        assert_eq!(
            strip_ansi("\u{1b}[32mtask: [a] echo a\u{1b}[0m"),
            "task: [a] echo a"
        );
    }

    fn feed(tracker: &mut Tracker<'_>, lines: &[&str]) -> Vec<EventBody> {
        let t0 = DateTime::from_timestamp(1_000, 0).expect("ts");
        lines
            .iter()
            .enumerate()
            .flat_map(|(i, line)| {
                let at = t0 + chrono::Duration::milliseconds(i as i64 * 10);
                tracker.observe(&classify(line), at)
            })
            .collect()
    }

    /// The exact stderr go-task 3.53 produced for `d: deps [b, c]`,
    /// `b: deps [a]`, `c: deps [b], status: [true]` — `b` and `a` run twice,
    /// concurrently — ending in `d` failing.
    #[test]
    fn duplicate_concurrent_executions_get_distinct_instances_and_parents() {
        let g = graph(vec![
            node("a", &[], &[]),
            node("b", &["a"], &[]),
            node("c", &["b"], &[]),
            node("d", &["b", "c"], &[]),
        ]);
        let index = GraphIndex::new(&g);
        let run_id = Uuid::now_v7();
        let mut tracker = Tracker::new(run_id, &index, "d");
        let events = feed(
            &mut tracker,
            &[
                "task: \"d\" started",
                "task: \"b\" started",
                "task: \"c\" started",
                "task: \"a\" started",
                "task: [a] echo a",
                "task: \"b\" started",
                "task: \"a\" started",
                "task: \"a\" finished",
                "task: \"a\" finished",
                "task: [b] echo b1",
                "task: \"b\" finished",
                "task: \"b\" finished",
                "task: status command true exited zero",
                "task: Task \"c\" is up to date",
                "task: [d] exit 3",
                "task: \"d\" failed: exit status 3",
                "task: Failed to run task \"d\": exit status 3",
            ],
        );

        let starts: Vec<(u32, &str, Option<u32>, Via)> = events
            .iter()
            .filter_map(|e| match e {
                EventBody::TaskStarted {
                    instance,
                    task,
                    parent,
                    via,
                    ..
                } => Some((*instance, task.as_str(), *parent, *via)),
                _ => None,
            })
            .collect();
        assert_eq!(
            starts,
            [
                (1, "d", None, Via::Root),
                (2, "b", Some(1), Via::Dep),
                (3, "c", Some(1), Via::Dep),
                (4, "a", Some(2), Via::Dep),
                (5, "b", Some(3), Via::Dep),
                (6, "a", Some(5), Via::Dep),
            ]
        );
        let outcomes: Vec<(u32, TaskOutcome)> = events
            .iter()
            .filter_map(|e| match e {
                EventBody::TaskFinished {
                    instance, outcome, ..
                } => Some((*instance, *outcome)),
                _ => None,
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                (4, TaskOutcome::Succeeded),
                (6, TaskOutcome::Succeeded),
                (2, TaskOutcome::Succeeded),
                (5, TaskOutcome::Succeeded),
                (3, TaskOutcome::UpToDate),
                (1, TaskOutcome::Failed),
            ]
        );

        let t_end = DateTime::from_timestamp(1_001, 0).expect("ts");
        let t_start = DateTime::from_timestamp(1_000, 0).expect("ts");
        let tail = tracker.finish(Some(201), t_start, t_end);
        assert_eq!(
            tail,
            [EventBody::RunFinished {
                run_id,
                outcome: RunOutcome::Failed,
                exit_code: Some(201),
                duration_ms: 1_000,
                error: Some("Failed to run task \"d\": exit status 3".into()),
            }]
        );
    }

    /// A dep fails, go-task aborts: the waiting parent failed (because of its
    /// child) and a sibling still running was cancelled.
    #[test]
    fn open_executions_at_exit_are_failed_or_cancelled_by_lineage() {
        let g = graph(vec![
            node("a", &[], &[]),
            node("b", &[], &[]),
            node("d", &["a", "b"], &[]),
        ]);
        let index = GraphIndex::new(&g);
        let mut tracker = Tracker::new(Uuid::now_v7(), &index, "d");
        feed(
            &mut tracker,
            &[
                "task: \"d\" started",
                "task: \"b\" started",
                "task: \"a\" started",
                "task: \"b\" failed: exit status 2",
            ],
        );
        let t = DateTime::from_timestamp(1_000, 0).expect("ts");
        let tail = tracker.finish(Some(201), t, t);
        let verdicts: Vec<(&str, TaskOutcome)> = tail
            .iter()
            .filter_map(|e| match e {
                EventBody::TaskFinished { task, outcome, .. } => Some((task.as_str(), *outcome)),
                _ => None,
            })
            .collect();
        assert_eq!(
            verdicts,
            [("a", TaskOutcome::Cancelled), ("d", TaskOutcome::Failed)]
        );
    }

    #[test]
    fn a_call_after_commands_started_is_attributed_as_call() {
        let g = graph(vec![node("x", &[], &[]), node("e", &["x"], &["x"])]);
        let index = GraphIndex::new(&g);
        let mut tracker = Tracker::new(Uuid::now_v7(), &index, "e");
        let events = feed(
            &mut tracker,
            &[
                "task: \"e\" started",
                "task: \"x\" started",
                "task: \"x\" finished",
                "task: [e] echo between",
                "task: \"x\" started",
            ],
        );
        let vias: Vec<Via> = events
            .iter()
            .filter_map(|e| match e {
                EventBody::TaskStarted { via, .. } => Some(*via),
                _ => None,
            })
            .collect();
        assert_eq!(vias, [Via::Root, Via::Dep, Via::Call]);
    }
}
