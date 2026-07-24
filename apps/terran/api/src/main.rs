//! terran API binary — thin entrypoint; logic lives in the `terran_api` library
//! (so integration tests can build the router directly).

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    terran_api::run().await?;
    Ok(())
}
