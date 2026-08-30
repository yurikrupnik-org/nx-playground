//! `todo_cli` — manage todos stored in NATS JetStream.
//!
//! The store is the `TODO_CLI` KV bucket; every mutation also publishes a
//! `todos.>` lifecycle event to the `TODOS` stream (same contract as
//! `todo_api`), so a running `todo_worker` picks up CLI changes.
//!
//! Connection resolution: `--nats-url` > `$NATS_URL` > `nats://localhost:4222`.

use clap::{Parser, Subcommand};
use core_config::env_or_default;
use domain_todo::{
    CreateTodo, NatsTodoPublisher, Todo, TodoEvent, TodoEventKind, TodoEventPublisher,
    TodoPriority, UpdateTodo,
};
use eyre::{Result, WrapErr, bail};
use todo_cli::{BUCKET, TodoStore, dlq_entries, purge_events, recent_events, redrive_dlq, status};

#[derive(Parser)]
#[command(
    name = "todo_cli",
    version,
    about = "Manage todos stored in NATS JetStream"
)]
struct Cli {
    /// NATS server URL (default: $NATS_URL, else nats://localhost:4222)
    #[arg(long, global = true)]
    nats_url: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a todo
    Add {
        title: String,
        #[arg(short, long, default_value = "")]
        description: String,
        /// low | medium | high
        #[arg(short, long, default_value = "medium", value_parser = parse_priority)]
        priority: TodoPriority,
    },
    /// List todos
    List {
        /// Only completed todos
        #[arg(long, conflicts_with = "pending")]
        completed: bool,
        /// Only pending todos
        #[arg(long)]
        pending: bool,
    },
    /// Edit a todo's title, description, or priority
    Edit {
        id: String,
        #[arg(short, long)]
        title: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
        /// low | medium | high
        #[arg(short, long, value_parser = parse_priority)]
        priority: Option<TodoPriority>,
    },
    /// Mark a todo as completed (accepts the short id shown by list)
    Done { id: String },
    /// Reopen a completed todo
    Undo { id: String },
    /// Delete a todo
    Rm { id: String },
    /// Show connection, store, and event-stream status
    Status,
    /// Inspect and manage the TODOS event stream
    #[command(subcommand)]
    Events(EventsCommand),
}

#[derive(Subcommand)]
enum EventsCommand {
    /// Show recent events, oldest first
    List {
        /// created | updated | completed | uncompleted | deleted
        #[arg(short, long, value_parser = parse_kind)]
        kind: Option<TodoEventKind>,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Show dead-lettered events (TODOS_DLQ)
    Dlq {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// Republish DLQ entries to their original subjects (run after a fix)
    Redrive {
        /// DLQ stream sequence to start from
        #[arg(long, default_value_t = 1)]
        from: u64,
        #[arg(short, long, default_value_t = 100)]
        limit: usize,
    },
    /// Delete every message in the TODOS stream (DLQ untouched)
    Purge {
        /// Required confirmation
        #[arg(long)]
        yes: bool,
    },
}

fn parse_kind(s: &str) -> Result<TodoEventKind, String> {
    match s.to_ascii_lowercase().as_str() {
        "created" => Ok(TodoEventKind::Created),
        "updated" => Ok(TodoEventKind::Updated),
        "completed" => Ok(TodoEventKind::Completed),
        "uncompleted" => Ok(TodoEventKind::Uncompleted),
        "deleted" => Ok(TodoEventKind::Deleted),
        other => Err(format!(
            "invalid kind '{other}' (created | updated | completed | uncompleted | deleted)"
        )),
    }
}

fn parse_priority(s: &str) -> Result<TodoPriority, String> {
    match s.to_ascii_lowercase().as_str() {
        "low" => Ok(TodoPriority::Low),
        "medium" => Ok(TodoPriority::Medium),
        "high" => Ok(TodoPriority::High),
        other => Err(format!("invalid priority '{other}' (low | medium | high)")),
    }
}

/// `list` row: short id (last 8 uuid chars — the random tail; the uuid v7
/// head is a timestamp shared across ids minted together), checkbox,
/// priority, title.
fn print_todo(todo: &Todo) {
    let mark = if todo.completed { "x" } else { " " };
    let id = todo.id.to_string();
    let short = &id[id.len() - 8..];
    println!("{short}  [{mark}] {:<6}  {}", todo.priority, todo.title);
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let nats_url = cli
        .nats_url
        .unwrap_or_else(|| env_or_default("NATS_URL", "nats://localhost:4222"));

    let client = messaging::nats::connect(&nats_url)
        .await
        .wrap_err_with(|| format!("connecting to NATS at {nats_url}"))?;
    let jetstream = async_nats::jetstream::new(client.clone());
    let store = TodoStore::open(&jetstream).await?;

    // Publisher only for mutations: creating it is what creates the TODOS
    // stream, and read-only commands must not write.
    let publisher = |js: async_nats::jetstream::Context| async {
        NatsTodoPublisher::new(js)
            .await
            .wrap_err("initializing TODOS event stream")
    };

    match cli.command {
        Command::Add {
            title,
            description,
            priority,
        } => {
            let todo = store
                .add(CreateTodo {
                    title,
                    description,
                    priority,
                })
                .await?;
            publisher(jetstream)
                .await?
                .publish(TodoEvent::from_todo(TodoEventKind::Created, &todo))
                .await?;
            print!("added ");
            print_todo(&todo);
        }
        Command::List { completed, pending } => {
            let todos = store.list().await?;
            let mut shown = 0;
            for todo in &todos {
                if completed && !todo.completed || pending && todo.completed {
                    continue;
                }
                print_todo(todo);
                shown += 1;
            }
            if shown == 0 {
                println!("no todos");
            }
        }
        Command::Edit {
            id,
            title,
            description,
            priority,
        } => {
            if title.is_none() && description.is_none() && priority.is_none() {
                bail!("nothing to change: pass --title, --description, or --priority");
            }
            let mut todo = store.resolve(&id).await?;
            todo.apply_update(UpdateTodo {
                title,
                description,
                completed: None,
                priority,
            });
            store.put(&todo).await?;
            publisher(jetstream)
                .await?
                .publish(TodoEvent::from_todo(TodoEventKind::Updated, &todo))
                .await?;
            print!("updated ");
            print_todo(&todo);
        }
        Command::Done { id } => {
            let mut todo = store.resolve(&id).await?;
            if todo.completed {
                println!("already completed: {}", todo.title);
                return Ok(());
            }
            todo.completed = true;
            todo.updated_at = chrono::Utc::now();
            store.put(&todo).await?;
            publisher(jetstream)
                .await?
                .publish(TodoEvent::from_todo(TodoEventKind::Completed, &todo))
                .await?;
            print!("done ");
            print_todo(&todo);
        }
        Command::Undo { id } => {
            let mut todo = store.resolve(&id).await?;
            if !todo.completed {
                println!("not completed: {}", todo.title);
                return Ok(());
            }
            todo.completed = false;
            todo.updated_at = chrono::Utc::now();
            store.put(&todo).await?;
            publisher(jetstream)
                .await?
                .publish(TodoEvent::from_todo(TodoEventKind::Uncompleted, &todo))
                .await?;
            print!("reopened ");
            print_todo(&todo);
        }
        Command::Rm { id } => {
            let todo = store.resolve(&id).await?;
            store.remove(todo.id).await?;
            publisher(jetstream)
                .await?
                .publish(TodoEvent::deleted(todo.id))
                .await?;
            print!("removed ");
            print_todo(&todo);
        }
        Command::Status => {
            let report = status(&jetstream, &store).await?;
            let info = client.server_info();
            println!(
                "NATS      {nats_url} · {} · server v{}",
                client.connection_state(),
                info.version
            );
            println!("Store     KV bucket {BUCKET} · {} todos", report.total);
            println!(
                "Todos     {} pending · {} completed",
                report.pending(),
                report.completed
            );
            println!(
                "Priority  {} high · {} medium · {} low",
                report.high, report.medium, report.low
            );
            match report.stream_messages {
                Some(n) => println!("Events    TODOS stream · {n} messages"),
                None => println!("Events    TODOS stream · not created yet"),
            }
            match report.dlq_messages {
                Some(n) => println!("DLQ       TODOS_DLQ · {n} entries"),
                None => println!("DLQ       TODOS_DLQ · empty (never created)"),
            }
        }
        Command::Events(events) => match events {
            EventsCommand::List { kind, limit } => {
                let events = recent_events(&jetstream, kind, limit).await?;
                if events.is_empty() {
                    println!("no events");
                }
                for recorded in events {
                    let e = &recorded.event;
                    let todo_id = e.todo_id.to_string();
                    let title = e.todo.as_ref().map_or("-", |t| t.title.as_str());
                    println!(
                        "{:>6}  {}  {:<11}  {}  {}",
                        recorded.sequence,
                        e.occurred_at.format("%Y-%m-%d %H:%M:%S"),
                        e.kind.subject_suffix(),
                        &todo_id[todo_id.len() - 8..],
                        title
                    );
                }
            }
            EventsCommand::Dlq { limit } => {
                let entries = dlq_entries(&jetstream, limit).await?;
                if entries.is_empty() {
                    println!("DLQ empty");
                }
                for (sequence, entry) in entries {
                    println!(
                        "{:>6}  {}  {}  deliveries={}  {}",
                        sequence,
                        entry.failed_at.format("%Y-%m-%d %H:%M:%S"),
                        entry.original_subject,
                        entry.delivery_count,
                        entry.error
                    );
                }
            }
            EventsCommand::Redrive { from, limit } => {
                let report = redrive_dlq(&jetstream, from, limit).await?;
                println!(
                    "redriven: {} republished · {} skipped",
                    report.republished, report.skipped
                );
            }
            EventsCommand::Purge { yes } => {
                if !yes {
                    bail!("purge deletes all TODOS events; re-run with --yes");
                }
                let purged = purge_events(&jetstream).await?;
                println!("purged {purged} events from TODOS");
            }
        },
    }

    Ok(())
}
