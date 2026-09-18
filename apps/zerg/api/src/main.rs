//! zerg API binary — thin entrypoint; logic lives in the `zerg_api` library
//! (so integration tests can build state and routers directly).

use core_config::tracing::install_color_eyre;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Install color-eyre first for colored error output (before any fallible operations)
    install_color_eyre();

    zerg_api::run().await
}
