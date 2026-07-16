//! Test publisher for email-nats worker
//!
//! Run with: cargo run -p zerg_email_nats --example publish_test

use email::EmailJob;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

    println!("Connecting to NATS at {}...", nats_url);
    let jetstream = messaging::nats::jetstream(&nats_url).await?;

    // Ensure stream exists
    println!("Creating/getting EMAILS stream...");
    let stream_config = async_nats::jetstream::stream::Config {
        name: "EMAILS".to_string(),
        subjects: vec!["emails.>".to_string()],
        retention: async_nats::jetstream::stream::RetentionPolicy::Limits,
        max_messages: 100_000,
        ..Default::default()
    };

    match jetstream.get_or_create_stream(stream_config).await {
        Ok(_) => println!("Stream EMAILS ready"),
        Err(e) => println!("Stream warning: {}", e),
    }

    // Create test email job using the actual EmailJob type
    let job = EmailJob::welcome("test@example.com", "Test User", "MyApp");

    let payload = serde_json::to_vec(&job)?;

    println!("Publishing email job: {:?}", job.id);
    println!("Email type: {:?}", job.email_type);
    println!("To: {}", job.to_email);

    let ack = jetstream
        .publish("emails.welcome", payload.into())
        .await?
        .await?;

    println!("Published! Stream sequence: {}", ack.sequence);
    println!("\nCheck the worker logs and Mailpit at http://localhost:8025");

    Ok(())
}
