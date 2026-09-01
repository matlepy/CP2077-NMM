use std::io::IsTerminal;

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::EnvFilter;

/// Initialize the global tracing subscriber.
///
/// Reads the log level from the `RUST_LOG` environment variable (default: `info`).
///
/// **Callers are responsible for NOT logging the raw API key.** Use [`redact_key`] when an
/// API key value needs to appear in an error message or log line.
pub fn init() {
    if tracing::dispatcher::has_been_set() {
        return;
    }

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive(LevelFilter::INFO.into()));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_thread_ids(false)
        .with_line_number(false)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

/// Returns the API key redacted for use in error messages or log lines.
pub fn redact_key(key: &str) -> String {
    if key.len() <= 8 {
        return "***REDACTED***".to_string();
    }
    let prefix = &key[..4];
    let suffix = &key[key.len() - 4..];
    format!("{prefix}***{suffix}")
}
