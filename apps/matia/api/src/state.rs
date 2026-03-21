/// Shared application state.
///
/// Lightweight for now — no database connections.
/// Add connections (PostgreSQL, Redis, etc.) as features require persistence.
#[derive(Clone)]
pub struct AppState {
    pub config: crate::config::Config,
}
