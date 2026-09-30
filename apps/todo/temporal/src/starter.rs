//! Drives one full todo lifecycle against a running `todo_temporal` worker:
//! start → query → update (priority high) → complete → await result.
//!
//! Usage: `todo_temporal_starter [title]`

use domain_todo::{CreateTodo, TodoPriority, UpdateTodo};
use temporalio_client::{
    Client, ClientOptions, Connection, WorkflowGetResultOptions, WorkflowQueryOptions,
    WorkflowSignalOptions, WorkflowStartOptions, envconfig::LoadClientConfigProfileOptions,
};
use todo_temporal::{TASK_QUEUE, TodoWorkflow, WORKFLOW_ID_PREFIX};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let title = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "try temporal".to_string());
    let todo_id = Uuid::new_v4();
    let workflow_id = format!("{WORKFLOW_ID_PREFIX}{todo_id}");

    let (conn_opts, client_opts) =
        ClientOptions::load_from_config(LoadClientConfigProfileOptions::default())?;
    let connection = Connection::connect(conn_opts).await?;
    let client = Client::new(connection, client_opts)?;

    let handle = client
        .start_workflow(
            TodoWorkflow::run,
            CreateTodo {
                title,
                description: "created by todo_temporal_starter".to_string(),
                priority: TodoPriority::Medium,
            },
            WorkflowStartOptions::new(TASK_QUEUE, workflow_id.as_str()).build(),
        )
        .await?;
    println!(
        "started todo {todo_id} (workflow {workflow_id}, run {:?})",
        handle.run_id()
    );

    let snapshot = handle
        .query(TodoWorkflow::get_todo, (), WorkflowQueryOptions::default())
        .await?;
    println!("query after create: {snapshot:?}");

    handle
        .signal(
            TodoWorkflow::update,
            UpdateTodo {
                priority: Some(TodoPriority::High),
                ..Default::default()
            },
            WorkflowSignalOptions::default(),
        )
        .await?;
    println!("signalled: update(priority=high)");

    handle
        .signal(TodoWorkflow::complete, (), WorkflowSignalOptions::default())
        .await?;
    println!("signalled: complete");

    let result = handle
        .get_result(WorkflowGetResultOptions::default())
        .await?;
    println!("workflow result: {result:?}");

    Ok(())
}
