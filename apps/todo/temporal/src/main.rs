//! Temporal worker hosting the todo lifecycle workflow + activities.
//!
//! Connects to Temporal via the standard env config (`TEMPORAL_ADDRESS`,
//! default `http://localhost:7233`) and to NATS via `NATS_URL`; like
//! `todo_api`, events degrade to a no-op publisher when NATS is unreachable.

use std::sync::Arc;

use core_config::{app_info, env_or_default, Environment};
use domain_todo::{NatsTodoPublisher, NoopTodoPublisher, TodoEventPublisher};
use eyre::{eyre, Result};
use temporalio_client::{
    envconfig::LoadClientConfigProfileOptions, Client, ClientOptions, Connection,
};
use temporalio_sdk::{Runtime, Worker, WorkerOptions};
use todo_temporal::{TodoActivities, TodoWorkflow, TASK_QUEUE};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    let environment = Environment::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&environment, app_info!());

    let nats_url = env_or_default("NATS_URL", "nats://localhost:4222");
    let publisher: Arc<dyn TodoEventPublisher> = match messaging::nats::jetstream(&nats_url).await {
        Ok(js) => match NatsTodoPublisher::new(js).await {
            Ok(p) => {
                info!(%nats_url, "publishing todo events to NATS JetStream (TODOS)");
                Arc::new(p)
            }
            Err(e) => {
                warn!(error = %e, "failed to init TODOS stream; events disabled");
                Arc::new(NoopTodoPublisher)
            }
        },
        Err(e) => {
            warn!(error = %e, %nats_url, "NATS unreachable; events disabled");
            Arc::new(NoopTodoPublisher)
        }
    };

    let runtime = Runtime::new_assume_tokio(Default::default())
        .map_err(|e| eyre!("init temporal runtime: {e}"))?;
    let (conn_opts, client_opts) =
        ClientOptions::load_from_config(LoadClientConfigProfileOptions::default())
            .map_err(|e| eyre!("load temporal client config: {e}"))?;
    let connection = Connection::connect(conn_opts)
        .await
        .map_err(|e| eyre!("connect to temporal: {e}"))?;
    let client =
        Client::new(connection, client_opts).map_err(|e| eyre!("build temporal client: {e}"))?;

    let worker_options = WorkerOptions::new(TASK_QUEUE)
        .register_workflow::<TodoWorkflow>()
        .map_err(|e| eyre!("register workflow: {e}"))?
        .register_activities(TodoActivities::new(publisher))
        .build();

    let mut worker =
        Worker::new(&runtime, client, worker_options).map_err(|e| eyre!("create worker: {e}"))?;
    info!(task_queue = TASK_QUEUE, "todo-temporal worker started");
    worker.run().await.map_err(|e| eyre!("worker run: {e}"))?;

    Ok(())
}
