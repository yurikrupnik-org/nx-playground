//! Vector gRPC Service - Entry Point

#[tokio::main]
async fn main() -> eyre::Result<()> {
    zerg_vector::run().await
}
