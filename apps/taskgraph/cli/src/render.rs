//! Plain-text, Mermaid and Graphviz renderings of graph, task, run and estimate views.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use contract_taskgraph::{Graph, RunOutcome, TaskOutcome, Via};
use domain_taskgraph::projection::{Execution, Run, RunSummary, TaskView};
use domain_taskgraph::{Estimate, TaskStats, TreeNode};

pub fn ms(value: u64) -> String {
    match value {
        0..1_000 => format!("{value}ms"),
        1_000..60_000 => format!("{:.1}s", value as f64 / 1000.0),
        _ => format!("{}m{:02}s", value / 60_000, (value % 60_000) / 1000),
    }
}

pub fn opt_ms(value: Option<u64>) -> String {
    value.map_or_else(|| "—".to_string(), ms)
}

fn task_outcome(outcome: Option<TaskOutcome>) -> &'static str {
    match outcome {
        Some(TaskOutcome::Succeeded) => "ok",
        Some(TaskOutcome::Failed) => "FAILED",
        Some(TaskOutcome::UpToDate) => "up-to-date",
        Some(TaskOutcome::Cancelled) => "cancelled",
        None => "running",
    }
}

fn run_outcome(outcome: Option<RunOutcome>) -> &'static str {
    match outcome {
        Some(RunOutcome::Succeeded) => "succeeded",
        Some(RunOutcome::Failed) => "FAILED",
        None => "running",
    }
}

fn via(via: Via) -> &'static str {
    match via {
        Via::Root => "",
        Via::Dep => " [dep]",
        Via::Call => " [call]",
    }
}

/// Short summary of a task's history for tree/list annotations.
pub fn stats_note(stats: &TaskStats) -> String {
    if stats.runs == 0 {
        return String::new();
    }
    format!(
        "p50 {} · self {} · {} runs · last {}",
        opt_ms(stats.p50_ms),
        opt_ms(stats.self_p50_ms),
        stats.runs,
        task_outcome(stats.last_outcome)
    )
}

/// Box-drawing tree; `note` annotates each resolved task.
pub fn tree(root: &TreeNode, note: &dyn Fn(&str) -> String) -> String {
    let mut out = String::new();
    line(&mut out, root, "", "", note);
    walk(&mut out, root, "", note);
    out
}

fn line(
    out: &mut String,
    node: &TreeNode,
    prefix: &str,
    branch: &str,
    note: &dyn Fn(&str) -> String,
) {
    let mut text = format!("{prefix}{branch}{}{}", node.task, via(node.via));
    if !node.resolved {
        text.push_str("  (unresolved)");
    } else {
        let n = note(&node.task);
        if !n.is_empty() {
            let _ = write!(text, "  {n}");
        }
    }
    if node.repeated {
        text.push_str("  ↑ (shown above)");
    }
    if node.truncated {
        text.push_str("  … (depth limit)");
    }
    out.push_str(&text);
    out.push('\n');
}

fn walk(out: &mut String, node: &TreeNode, prefix: &str, note: &dyn Fn(&str) -> String) {
    let count = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        let last = i + 1 == count;
        line(out, child, prefix, if last { "└── " } else { "├── " }, note);
        let next = format!("{prefix}{}", if last { "    " } else { "│   " });
        walk(out, child, &next, note);
    }
}

/// Tasks to draw: the whole graph, or `focus` and everything it can run.
fn edges_in(graph: &Graph, focus: Option<&BTreeSet<String>>) -> Vec<(String, String, Via)> {
    let mut edges = Vec::new();
    for task in &graph.tasks {
        if focus.is_some_and(|f| !f.contains(&task.name)) {
            continue;
        }
        for (kind, list) in [(Via::Dep, &task.deps), (Via::Call, &task.calls)] {
            for r in list.iter().filter(|r| r.resolved) {
                edges.push((task.name.clone(), r.name.clone(), kind));
            }
        }
    }
    edges
}

fn node_ids(graph: &Graph) -> HashMap<&str, String> {
    graph
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.as_str(), format!("t{i}")))
        .collect()
}

pub fn mermaid(graph: &Graph, focus: Option<&BTreeSet<String>>) -> String {
    let ids = node_ids(graph);
    let mut out = String::from("graph TD\n");
    for task in &graph.tasks {
        if focus.is_none_or(|f| f.contains(&task.name))
            && let Some(id) = ids.get(task.name.as_str())
        {
            let _ = writeln!(out, "  {id}[\"{}\"]", task.name.replace('"', "'"));
        }
    }
    for (from, to, kind) in edges_in(graph, focus) {
        if let (Some(a), Some(b)) = (ids.get(from.as_str()), ids.get(to.as_str())) {
            let arrow = if kind == Via::Call {
                "-. call .->"
            } else {
                "-->"
            };
            let _ = writeln!(out, "  {a} {arrow} {b}");
        }
    }
    out
}

pub fn dot(graph: &Graph, focus: Option<&BTreeSet<String>>) -> String {
    let mut out = String::from("digraph taskgraph {\n  rankdir=LR;\n  node [shape=box];\n");
    for task in &graph.tasks {
        if focus.is_none_or(|f| f.contains(&task.name)) {
            let _ = writeln!(out, "  {:?};", task.name);
        }
    }
    for (from, to, kind) in edges_in(graph, focus) {
        let style = if kind == Via::Call {
            " [style=dashed, label=\"call\"]"
        } else {
            ""
        };
        let _ = writeln!(out, "  {from:?} -> {to:?}{style};");
    }
    out.push_str("}\n");
    out
}

/// Left-aligned columns; the last column is not padded.
pub fn table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(cell.chars().count());
            }
        }
    }
    let mut out = String::new();
    let mut emit = |cells: Vec<&str>| {
        let last = cells.len().saturating_sub(1);
        for (i, cell) in cells.iter().enumerate() {
            if i == last {
                out.push_str(cell);
            } else {
                let pad = widths[i] - cell.chars().count();
                let _ = write!(out, "{cell}{}  ", " ".repeat(pad));
            }
        }
        out.push('\n');
    };
    emit(header.to_vec());
    for row in rows {
        emit(row.iter().map(String::as_str).collect());
    }
    out
}

fn section(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n{title}:");
    for item in items {
        let _ = writeln!(out, "  {item}");
    }
}

pub fn task_view(view: &TaskView, note: &dyn Fn(&str) -> String) -> String {
    let t = &view.task;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}{}",
        t.name,
        if t.internal { "  (internal)" } else { "" }
    );
    if let Some(desc) = &t.desc {
        let _ = writeln!(out, "  {desc}");
    }
    let _ = writeln!(out, "  defined in {}", t.taskfile);
    if !t.aliases.is_empty() {
        let _ = writeln!(out, "  aliases: {}", t.aliases.join(", "));
    }
    if let Some(run) = &t.run {
        let _ = writeln!(out, "  run: {run}");
    }
    if let Some(summary) = &t.summary {
        let _ = writeln!(out, "\n{}", summary.trim_end());
    }

    let refs = |list: &[contract_taskgraph::TaskRef]| -> Vec<String> {
        list.iter()
            .map(|r| {
                if r.resolved {
                    r.name.clone()
                } else {
                    format!("{}  (unresolved)", r.name)
                }
            })
            .collect()
    };
    section(&mut out, "deps (concurrent)", &refs(&t.deps));
    section(&mut out, "calls (sequential)", &refs(&t.calls));
    section(&mut out, "cmds", &t.cmds);
    section(&mut out, "sources", &t.sources);
    section(&mut out, "generates", &t.generates);
    section(&mut out, "status", &t.status);
    section(&mut out, "preconditions", &t.preconditions);
    section(&mut out, "requires vars", &t.requires);
    let dependents: Vec<String> = view
        .dependents
        .iter()
        .map(|d| format!("{}{}", d.task, via(d.via)))
        .collect();
    section(&mut out, "used by", &dependents);
    if view.transitive_dependents.len() > view.dependents.len() {
        let _ = writeln!(
            out,
            "  … {} tasks transitively",
            view.transitive_dependents.len()
        );
    }

    let _ = writeln!(
        out,
        "\ndrill-down ({} tasks reachable):",
        view.closure.len()
    );
    out.push_str(&tree(&view.tree, note));

    let s = &view.stats;
    if s.runs > 0 {
        let _ = writeln!(
            out,
            "\nhistory: {} runs — {} ok, {} up-to-date, {} failed, {} cancelled",
            s.runs, s.succeeded, s.up_to_date, s.failed, s.cancelled
        );
        let _ = writeln!(
            out,
            "  duration p50 {} · p90 {} · max {}   self p50 {} · p90 {}",
            opt_ms(s.p50_ms),
            opt_ms(s.p90_ms),
            opt_ms(s.max_ms),
            opt_ms(s.self_p50_ms),
            opt_ms(s.self_p90_ms)
        );
    } else {
        out.push_str("\nhistory: none yet (run it with `taskgraph run`)\n");
    }
    if let Some(est) = &view.estimate {
        out.push_str(&estimate_line(est));
    }
    out
}

pub fn estimate_line(est: &Estimate) -> String {
    let mut out = format!(
        "estimate: ~{} (pessimistic {}) over {} tasks\n  critical path: {}\n",
        ms(est.expected_ms),
        ms(est.pessimistic_ms),
        est.tasks,
        est.critical_path.join(" → ")
    );
    if !est.unknown.is_empty() {
        let _ = writeln!(
            out,
            "  no history for {} task(s), counted as 0: {}",
            est.unknown.len(),
            est.unknown.join(", ")
        );
    }
    out
}

pub fn runs_table(runs: &[RunSummary]) -> String {
    let rows: Vec<Vec<String>> = runs
        .iter()
        .map(|r| {
            vec![
                r.run_id.to_string(),
                r.target.clone(),
                run_outcome(r.outcome).to_string(),
                opt_ms(r.duration_ms),
                opt_ms(r.estimate_ms),
                format!("{}/{}", r.failed, r.executions),
                r.started_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                format!("{}@{}", r.user, r.host),
            ]
        })
        .collect();
    table(
        &[
            "RUN",
            "TARGET",
            "OUTCOME",
            "TOOK",
            "EXPECTED",
            "FAILED/TASKS",
            "STARTED (UTC)",
            "BY",
        ],
        &rows,
    )
}

/// Execution tree of one run, children under the execution that pulled them in.
pub fn run_detail(run: &Run, commands: bool, trace_url: Option<&str>) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "run {}  {}  target {}  took {}  (expected {})",
        run.run_id,
        run_outcome(run.outcome),
        run.target,
        opt_ms(run.duration_ms),
        opt_ms(run.estimate_ms)
    );
    let _ = writeln!(
        out,
        "  {}@{} in {}  started {}",
        run.user,
        run.host,
        run.cwd,
        run.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    if !run.args.is_empty() {
        let _ = writeln!(out, "  args: {}", run.args.join(" "));
    }
    if let Some(error) = &run.error {
        let _ = writeln!(out, "  error: {error}");
    }
    if let Some(trace) = &run.trace {
        match trace_url {
            Some(template) => {
                let _ = writeln!(
                    out,
                    "  trace: {}",
                    template.replace("{trace_id}", &trace.trace_id)
                );
            }
            None => {
                let _ = writeln!(out, "  trace: {}", trace.trace_id);
            }
        }
    }
    out.push('\n');
    let roots: Vec<&Execution> = run
        .executions
        .iter()
        .filter(|e| {
            e.parent
                .is_none_or(|p| !run.executions.iter().any(|x| x.instance == p))
        })
        .collect();
    let count = roots.len();
    for (i, root) in roots.into_iter().enumerate() {
        execution(&mut out, run, root, "", i + 1 == count, true, commands);
    }
    out
}

fn execution(
    out: &mut String,
    run: &Run,
    e: &Execution,
    prefix: &str,
    last: bool,
    top: bool,
    commands: bool,
) {
    let branch = match (top, last) {
        (true, _) => "",
        (false, true) => "└── ",
        (false, false) => "├── ",
    };
    let offset = crate::render::ms(domain_taskgraph::observe::millis_between(
        run.started_at,
        e.started_at,
    ));
    let _ = write!(
        out,
        "{prefix}{branch}{}{}  {}  took {}  self {}  (+{offset})",
        e.task,
        via(e.via),
        task_outcome(e.outcome),
        opt_ms(e.duration_ms),
        opt_ms(e.self_ms),
    );
    if let Some(error) = &e.error {
        let _ = write!(out, "  — {error}");
    }
    out.push('\n');
    let child_prefix = if top {
        prefix.to_string()
    } else {
        format!("{prefix}{}", if last { "    " } else { "│   " })
    };
    if commands {
        for c in &e.commands {
            let _ = writeln!(out, "{child_prefix}  $ {}", c.command);
        }
    }
    let children: Vec<&Execution> = run
        .executions
        .iter()
        .filter(|x| x.parent == Some(e.instance))
        .collect();
    let count = children.len();
    for (i, child) in children.into_iter().enumerate() {
        execution(
            out,
            run,
            child,
            &child_prefix,
            i + 1 == count,
            false,
            commands,
        );
    }
}
