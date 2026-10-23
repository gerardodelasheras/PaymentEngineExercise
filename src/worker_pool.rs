//! Worker-pool actor runtime: channels, workers, dispatch, and collection.

use std::collections::HashMap;

use tokio::sync::{mpsc, oneshot};

use crate::account::Account;
use crate::client::ClientId;
use crate::processing_error::ProcessingError;
use crate::transaction::{Transaction, TransactionProcessor};

enum Command {
    Process(Transaction),
    TakeAccounts(oneshot::Sender<HashMap<ClientId, Account>>),
}

/// Maximum number of pending commands per worker channel.
const QUEUE_CAPACITY: usize = 256;

/// A fixed pool of actor workers, each owning its clients' state.
#[derive(Clone)]
pub(crate) struct WorkerPool {
    senders: Vec<mpsc::Sender<Command>>,
}

impl WorkerPool {
    /// Spawn one worker per logical CPU core.
    pub(crate) fn new() -> Self {
        Self::with_worker_count(num_workers())
    }

    fn with_worker_count(worker_count: usize) -> Self {
        let mut senders = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let (sender, mut receiver) = mpsc::channel::<Command>(QUEUE_CAPACITY);
            tokio::spawn(async move {
                let mut processor = TransactionProcessor::new();
                while let Some(command) = receiver.recv().await {
                    match command {
                        Command::Process(transaction) => {
                            if let Err(error) = processor.process(&transaction) {
                                tracing::debug!(
                                    transaction_id = transaction.transaction_id,
                                    client = transaction.client,
                                    error = %error,
                                    "skipped transaction"
                                );
                            }
                        }
                        Command::TakeAccounts(response) => {
                            let _ = response.send(std::mem::take(&mut processor.accounts));
                        }
                    }
                }
            });
            senders.push(sender);
        }
        Self { senders }
    }

    /// Number of workers in the pool.
    pub(crate) fn worker_count(&self) -> usize {
        self.senders.len()
    }

    /// Remaining capacity of each worker's queue.
    pub(crate) fn capacities(&self) -> Vec<usize> {
        self.senders
            .iter()
            .map(|sender| sender.capacity())
            .collect()
    }

    /// Dispatch a transaction to the given worker.
    pub(crate) fn dispatch_to(
        &self,
        worker: usize,
        transaction: Transaction,
    ) -> Result<(), ProcessingError> {
        self.senders[worker]
            .blocking_send(Command::Process(transaction))
            .map_err(|_| ProcessingError::WorkerUnavailable)
    }

    /// Take each worker's accounts and merge them into one map.
    pub(crate) async fn take_accounts(self) -> Result<HashMap<ClientId, Account>, ProcessingError> {
        let mut accounts = HashMap::new();
        for sender in &self.senders {
            let (response_tx, response_rx) = oneshot::channel();
            sender
                .send(Command::TakeAccounts(response_tx))
                .await
                .map_err(|_| ProcessingError::WorkerUnavailable)?;
            let worker_accounts = response_rx
                .await
                .map_err(|_| ProcessingError::WorkerUnavailable)?;
            accounts.extend(worker_accounts);
        }
        Ok(accounts)
    }
}

/// Number of worker tasks to spawn, one per logical CPU core.
fn num_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Amount;
    use crate::transaction::TransactionType;

    fn deposit(client: ClientId, transaction_id: u32) -> Transaction {
        Transaction {
            transaction_type: TransactionType::Deposit,
            client,
            transaction_id,
            amount: Some(Amount(100)),
        }
    }

    fn pool_with(sender: mpsc::Sender<Command>) -> WorkerPool {
        WorkerPool {
            senders: vec![sender],
        }
    }

    #[tokio::test]
    async fn capacities_report_remaining_queue_slots() {
        let (sender_a, _receiver_a) = mpsc::channel::<Command>(2);
        let (sender_b, _receiver_b) = mpsc::channel::<Command>(2);
        let (response_tx, _response_rx) = oneshot::channel();
        sender_a
            .send(Command::TakeAccounts(response_tx))
            .await
            .expect("queue accepts command");
        let pool = WorkerPool {
            senders: vec![sender_a, sender_b],
        };

        assert_eq!(pool.capacities(), vec![1, 2]);
    }

    #[test]
    fn dispatch_to_reports_closed_channel() {
        let (sender, receiver) = mpsc::channel::<Command>(1);
        drop(receiver);
        let pool = pool_with(sender);

        let result = pool.dispatch_to(0, deposit(0, 1));

        assert!(matches!(result, Err(ProcessingError::WorkerUnavailable)));
    }

    #[tokio::test]
    async fn take_accounts_reports_closed_channel() {
        let (sender, receiver) = mpsc::channel::<Command>(1);
        drop(receiver);
        let pool = pool_with(sender);

        let result = pool.take_accounts().await;

        assert!(matches!(result, Err(ProcessingError::WorkerUnavailable)));
    }

    #[tokio::test]
    async fn dispatch_to_applies_backpressure_when_queue_is_full() {
        let (sender, mut receiver) = mpsc::channel::<Command>(1);
        let (response_tx, _response_rx) = oneshot::channel();
        sender
            .send(Command::TakeAccounts(response_tx))
            .await
            .expect("queue accepts the dummy command");
        let pool = pool_with(sender);

        let handle = std::thread::spawn(move || pool.dispatch_to(0, deposit(0, 1)));

        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            !handle.is_finished(),
            "dispatch should block while the bounded queue is full"
        );

        let _ = receiver.recv().await.expect("dummy command");
        let result = handle.join().expect("dispatch thread panicked");

        assert!(result.is_ok());
    }
}
