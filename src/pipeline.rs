use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};

use tracing::trace;

use crate::account::report;
use crate::client::client_id::ClientId;
use crate::client::dispatcher::ClientDispatcher;
use crate::processing_error::ProcessingError;
use crate::transaction::parser;
use crate::worker_pool::WorkerPool;

/// Reads `path`, applies every transaction, and writes the accounts CSV to `out`.
pub async fn process<W: Write>(path: &Path, out: &mut W) -> Result<(), ProcessingError> {
    let pool = WorkerPool::new();
    let path = PathBuf::from(path);

    let reader = tokio::task::spawn_blocking(move || {
        let result = read_and_dispatch(&path, &pool);
        (pool, result)
    });

    let (pool, dispatch_result) = reader.await.map_err(|error| {
        ProcessingError::WorkerUnavailable(format!("reader task failed: {error}"))
    })?;
    let accounts_result = pool.take_accounts().await;

    match (dispatch_result, accounts_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(accounts)) => report::write_accounts(out, &accounts),
    }
}

fn read_and_dispatch(path: &Path, pool: &WorkerPool) -> Result<(), ProcessingError> {
    let file = File::open(path).map_err(|source| ProcessingError::FileOpen {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = BufReader::new(file);
    let transactions = parser::transactions(reader)?;
    let mut dispatcher = ClientDispatcher::new(pool.worker_count());

    for item in transactions {
        match item {
            Ok(transaction) => {
                let worker =
                    routed_worker(&mut dispatcher, transaction.client, || pool.capacities());
                pool.dispatch_to(worker, transaction)?;
            }
            Err(error) => {
                if error.is_recoverable() {
                    trace!(error = %error, "skipped transaction row");
                    continue;
                }
                return Err(error);
            }
        }
    }

    Ok(())
}

/// Returns the pinned worker for `client`, requesting capacities only when the
/// client has not been assigned yet.
fn routed_worker<F>(
    dispatcher: &mut ClientDispatcher,
    client: ClientId,
    capacity_supplier: F,
) -> usize
where
    F: FnOnce() -> Vec<usize>,
{
    match dispatcher.worker_of(client) {
        Some(worker) => worker,
        None => dispatcher.worker_for(client, &capacity_supplier()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn pinned_clients_do_not_request_capacities() {
        let mut dispatcher = ClientDispatcher::new(2);
        let pinned = dispatcher.worker_for(7, &[10, 10]);
        let supplier_calls = Cell::new(0);

        let routed = routed_worker(&mut dispatcher, 7, || {
            supplier_calls.set(supplier_calls.get() + 1);
            vec![10, 10]
        });

        assert_eq!(routed, pinned);
        assert_eq!(supplier_calls.get(), 0);
    }

    #[test]
    fn new_clients_request_capacities_once() {
        let mut dispatcher = ClientDispatcher::new(2);
        let supplier_calls = Cell::new(0);

        let routed = routed_worker(&mut dispatcher, 7, || {
            supplier_calls.set(supplier_calls.get() + 1);
            vec![10, 10]
        });

        assert_eq!(supplier_calls.get(), 1);
        assert_eq!(dispatcher.worker_of(7), Some(routed));
    }
}
