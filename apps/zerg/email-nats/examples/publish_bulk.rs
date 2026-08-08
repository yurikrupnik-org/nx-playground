//! Bulk publisher for load-testing the email worker.
//!
//! Publishes N distinct welcome jobs onto `EMAILS` as fast as the server will ack,
//! then prints the publish-side throughput. The delivery-side assertion lives in
//! `just email-scale-check`, which counts what actually reached MailHog.
//!
//! Run with: `cargo run -p zerg_email_nats --example publish_bulk -- 500`
//!
//! Each job gets a unique recipient (`scale-{i}@example.com`) so a duplicate
//! delivery is visible as a repeated address rather than hiding in a total.

use std::time::Instant;

use email::EmailJob;
use email::EmailNatsStream;
use messaging::nats::{stream_config_for, StreamConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "100".to_string())
        .parse()?;

    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

    let jetstream = messaging::nats::jetstream(&nats_url).await?;

    // Same builder the worker uses. Hardcoding a config here is how a publisher
    // silently recreates the stream with the wrong retention and restores fan-out.
    jetstream
        .get_or_create_stream(stream_config_for(
            EmailNatsStream::STREAM_NAME,
            EmailNatsStream::SUBJECT,
            EmailNatsStream::KIND,
        ))
        .await?;

    println!("Publishing {count} jobs to {}...", EmailNatsStream::STREAM_NAME);
    let started = Instant::now();

    // Publish in bounded batches: send a chunk, then drain its acks before sending
    // the next. Awaiting every ack individually would measure round-trip latency
    // instead of throughput, but letting acks accumulate without limit deadlocks the
    // client once the outstanding-ack window fills (observed at ~5k on this setup:
    // the publisher wedges while every message it did send is consumed normally).
    const IN_FLIGHT: usize = 500;
    let mut acks = Vec::with_capacity(IN_FLIGHT);
    for i in 0..count {
        let job = EmailJob::welcome(
            format!("scale-{i}@example.com"),
            format!("Scale User {i}"),
            "MyApp",
        );
        let payload = serde_json::to_vec(&job)?;
        acks.push(jetstream.publish("emails.welcome", payload.into()).await?);

        if acks.len() == IN_FLIGHT {
            for ack in acks.drain(..) {
                ack.await?;
            }
        }
    }
    for ack in acks.drain(..) {
        ack.await?;
    }

    let elapsed = started.elapsed();
    println!(
        "published {count} jobs in {:.2}s ({:.0} msg/s)",
        elapsed.as_secs_f64(),
        count as f64 / elapsed.as_secs_f64()
    );

    Ok(())
}
