//! Command-line entry point for the in-memory streaming processor.
//!
//! Usage: `transaction_processor <transactions.csv>`.
//! The accounts CSV is written to stdout; diagnostics are written to stderr.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    let mut arguments = std::env::args_os().skip(1);
    let path = match (arguments.next(), arguments.next()) {
        (Some(path), None) => PathBuf::from(path),
        _ => {
            eprintln!("usage: transaction_processor <transactions.csv>");
            return ExitCode::from(2);
        }
    };

    let mut stdout = std::io::stdout().lock();
    match transaction_processor::pipeline::process(&path, &mut stdout).await {
        Ok(()) => {
            let _ = stdout.flush();
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Installs stderr tracing, defaulting to warnings. Release builds clamp
/// `RUST_LOG` because trace and debug events are compiled out of release.
fn init_tracing() {
    #[cfg(not(debug_assertions))]
    {
        // Trace and debug events are compiled out of release builds. Clamp the
        // request so the filter does not warn about statically disabled levels.
        if std::env::var_os("RUST_LOG").is_some() {
            std::env::set_var("RUST_LOG", "info");
        }
    }

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
