//! Streaming pipeline: CSV ingestion, dispatch, and account output.

use std::io::{Read, Write};

use crate::account::report;
use crate::client::ClientDispatcher;
use crate::processing_error::ProcessingError;
use crate::transaction::parser;
use crate::worker_pool::WorkerPool;

/// Process a transaction CSV from `reader` and write the accounts CSV to
/// `writer`. Rows that fail boundary validation or a state transition are
/// skipped silently; diagnostics go to stderr via `tracing`. Loss of a
/// producer or worker channel returns [`ProcessingError::WorkerUnavailable`].
pub async fn process<R: Read + Send + 'static, W: Write>(
    reader: R,
    writer: &mut W,
) -> Result<(), ProcessingError> {
    let pool = WorkerPool::new();
    let producer_pool = pool.clone();
    let producer = tokio::task::spawn_blocking(move || read_and_dispatch(reader, &producer_pool));
    let producer_result = producer
        .await
        .map_err(|_| ProcessingError::WorkerUnavailable)?;
    producer_result?;
    let accounts = pool.take_accounts().await?;
    report::write_accounts(writer, &accounts)
}

/// Parse the CSV stream and dispatch each valid transaction to the worker that
/// owns its client. Malformed rows are skipped with a `warn` diagnostic.
fn read_and_dispatch<R: Read>(reader: R, pool: &WorkerPool) -> Result<(), ProcessingError> {
    let mut dispatcher = ClientDispatcher::new(pool.worker_count());

    for result in parser::transactions(reader) {
        match result {
            Ok(transaction) => {
                let worker = match dispatcher.assigned_worker(transaction.client) {
                    Some(worker) => worker,
                    None => {
                        let capacities = pool.capacities();
                        dispatcher.assign(transaction.client, &capacities)
                    }
                };
                pool.dispatch_to(worker, transaction)?;
            }
            Err(error) => {
                tracing::warn!(error = %error, "skipped malformed transaction row");
            }
        }
    }
    Ok(())
}
