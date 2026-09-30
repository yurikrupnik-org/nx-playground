//! Restate service endpoint hosting [`TodoObject`].
//!
//! Push model, unlike Temporal's polling worker: this process serves HTTP/2 on
//! `RESTATE_ENDPOINT_ADDR` (default `0.0.0.0:9080`) and the Restate server calls
//! into it. Run a server, start this, then register it once:
//!
//! ```text
//! docker compose -f manifests/dockers/compose.yaml up -d restate
//! cargo run -p todo_restate
//! curl localhost:9070/deployments --json '{"uri":"http://host.docker.internal:9080"}'
//! ```
//!
//! Events go to NATS at `NATS_URL`; like `todo_api` and `todo_temporal`, they
//! degrade to a no-op publisher when NATS is unreachable at startup.

use std::net::SocketAddr;
use std::sync::Arc;

use core_config::{Environment, app_info, env_or_default};
use domain_todo::{NatsTodoPublisher, NoopTodoPublisher, TodoEventPublisher};
use eyre::{Result, WrapErr};
use restate_sdk::prelude::{Endpoint, HttpServer};
use todo_restate::TodoObject;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    let environment = Environment::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&environment, app_info!());

    let addr: SocketAddr = env_or_default("RESTATE_ENDPOINT_ADDR", "0.0.0.0:9080")
        .parse()
        .wrap_err("RESTATE_ENDPOINT_ADDR must be host:port")?;

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

    info!(%addr, "todo-restate endpoint listening");
    HttpServer::new(Endpoint::builder().bind(TodoObject::new(publisher)).build())
        .listen_and_serve(addr)
        .await;

    Ok(())
}
