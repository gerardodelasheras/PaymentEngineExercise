//! Command-line entry point for the in-memory streaming processor.
//!
//! Reads a transactions CSV from the sole positional argument and writes the
//! resulting accounts CSV to standard output.

use std::env;
use std::fs::File;
use std::io::{self, BufReader};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: transaction_processor <transactions.csv>");
        std::process::exit(1);
    }

    let transactions_file = match File::open(&args[1]) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error opening {}: {}", args[1], e);
            std::process::exit(1);
        }
    };

    let transactions_file_reader = BufReader::new(transactions_file);
    let stdout = io::stdout();
    let mut out = stdout.lock();

    if let Err(e) =
        transaction_processor::pipeline::process(transactions_file_reader, &mut out).await
    {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
