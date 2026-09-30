//! Per-identity feature flags backed by Flagsmith, with hardcoded fallbacks.
//!
//! The crate wraps two Flagsmith SDK endpoints (`/flags/` and `/identities/`)
//! over async `reqwest`. It deliberately does not use the official `flagsmith`
//! crate: that client is `reqwest::blocking` and panics when driven from an
//! async runtime.
//!
//! Two invariants shape the whole API:
//!
//! 1. **A flag lookup never fails.** [`FlagClient::flags`] returns a
//!    [`FlagSet`], not a `Result`. If Flagsmith is unconfigured, unreachable,
//!    slow, or answers garbage, the caller gets the hardcoded [`FlagDefaults`]
//!    tagged [`FlagSource::Defaults`].
//! 2. **Every known flag is always present.** Defaults are the floor and remote
//!    values are merged over them, so call sites never branch on "missing".
//!
//! ```no_run
//! use core_config::FromEnv;
//! use feature_flags::{FlagClient, FlagDefaults, FlagsmithConfig, Identity};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let defaults = FlagDefaults::new()
//!     .bool("todo_write", true)
//!     .int("todo_max_items", -1);
//! let client = FlagClient::new(FlagsmithConfig::from_env()?, defaults)?;
//!
//! let identity = Identity::new("yuri").with_trait("app", "htmx");
//! let flags = client.flags(Some(&identity)).await;
//! if flags.enabled("todo_write") {
//!     // ...
//! }
//! # Ok(())
//! # }
//! ```

mod client;
mod config;
mod error;
mod flags;
mod identity;

pub use client::FlagClient;
pub use config::FlagsmithConfig;
pub use error::FlagsError;
pub use flags::{FlagDefaults, FlagSet, FlagSource, ResolvedFlag};
pub use identity::Identity;

/// Serializes tests that mutate process-global environment variables.
///
/// `temp_env` sets and restores the real process environment, but Rust runs a
/// crate's unit tests concurrently in one process — so two tests touching the
/// same key race: one restores it while the other is still asserting on it.
/// Every test that calls `temp_env` MUST hold this guard.
#[cfg(test)]
pub(crate) mod test_env {
    use parking_lot::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Bind the guard for the whole test body: `let _env = test_env::guard();`.
    pub(crate) fn guard() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock()
    }
}
