//! Logging initialization for the CLI.

use tracing_subscriber::EnvFilter;

/// Initialize the `tracing` subscriber, honoring `REGISTER_LOG` (falls back
/// to `info`).
pub(crate) fn init() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            EnvFilter::try_from_env("REGISTER_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}
