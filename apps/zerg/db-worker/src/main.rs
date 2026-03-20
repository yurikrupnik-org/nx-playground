//! Multi-Database Worker Service (Dapr + NATS JetStream)
//!
//! Binary entry point for the event-driven DB worker.

#[tokio::main]
async fn main() {
    if let Err(e) = zerg_db_worker::run().await {
        eprintln!("Fatal error: {:#}", e);
        std::process::exit(1);
    }
}
