//! Email Worker Service (NATS JetStream)
//!
//! Binary entry point for the NATS-based email worker.

#[tokio::main]
async fn main() {
    if let Err(e) = zerg_email_nats::run().await {
        eprintln!("Fatal error: {e:#}");
        std::process::exit(1);
    }
}
