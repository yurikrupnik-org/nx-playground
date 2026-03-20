//! MongoDB connection utilities

use mongodb::{Client, Database, options::ClientOptions};

/// Connect to MongoDB and return the database handle
pub async fn connect(url: &str, database_name: &str) -> Result<Database, mongodb::error::Error> {
    let options = ClientOptions::parse(url).await?;
    let client = Client::with_options(options)?;

    // Ping to verify connectivity
    client
        .database("admin")
        .run_command(mongodb::bson::doc! { "ping": 1 })
        .await?;

    tracing::info!(database = database_name, "Connected to MongoDB");
    Ok(client.database(database_name))
}
