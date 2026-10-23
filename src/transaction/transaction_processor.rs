//! The transaction processor for one worker: applies transactions and mutates
//! the accounts and transaction index it owns.

use std::collections::HashMap;

use super::transaction_entry::TransactionEntry;
use crate::account::Account;
use crate::amount::Amount;
use crate::client::ClientId;
use crate::processing_error::ProcessingError;
use crate::transaction::{Transaction, TransactionId, TransactionType};

/// Applies transactions against the accounts and transaction index owned by a
/// single worker.
#[derive(Debug, Default)]
pub struct TransactionProcessor {
    pub accounts: HashMap<ClientId, Account>,
    entries: HashMap<TransactionId, TransactionEntry>,
}

impl TransactionProcessor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one transaction, mutating account and transaction state in place.
    /// Returns `Err` (without mutating balances or the transaction index) when
    /// the row must be skipped. A skipped operation on a previously-unknown
    /// client still initializes a zeroed account.
    pub fn process(&mut self, transaction: &Transaction) -> Result<(), ProcessingError> {
        if self.accounts.entry(transaction.client).or_default().locked {
            return Err(ProcessingError::AccountLocked);
        }

        match transaction.transaction_type {
            TransactionType::Deposit => self.process_deposit(transaction),
            TransactionType::Withdrawal => self.process_withdrawal(transaction),
            TransactionType::Dispute => self.process_dispute(transaction),
            TransactionType::Resolve => self.process_resolve(transaction),
            TransactionType::Chargeback => self.process_chargeback(transaction),
        }
    }

    fn process_deposit(&mut self, transaction: &Transaction) -> Result<(), ProcessingError> {
        let amount = transaction.amount.ok_or(ProcessingError::MissingAmount)?;
        let account = self.accounts.entry(transaction.client).or_default();
        account.credit(amount)?;
        self.record_settled(transaction, amount);
        Ok(())
    }

    fn process_withdrawal(&mut self, transaction: &Transaction) -> Result<(), ProcessingError> {
        let amount = transaction.amount.ok_or(ProcessingError::MissingAmount)?;
        let account = self.accounts.entry(transaction.client).or_default();
        account.debit(amount)?;
        self.record_settled(transaction, amount);
        Ok(())
    }

    fn record_settled(&mut self, transaction: &Transaction, amount: Amount) {
        self.entries.insert(
            transaction.transaction_id,
            TransactionEntry::Settled {
                client: transaction.client,
                amount,
            },
        );
    }

    fn process_dispute(&mut self, transaction: &Transaction) -> Result<(), ProcessingError> {
        let (client, amount) = match self.entries.get(&transaction.transaction_id) {
            Some(TransactionEntry::Settled { client, amount }) => (*client, *amount),
            _ => return Err(ProcessingError::UnknownTx),
        };
        if client != transaction.client {
            return Err(ProcessingError::NotOwner);
        }
        let account = self.accounts.entry(transaction.client).or_default();
        account.hold(amount)?;
        self.entries.insert(
            transaction.transaction_id,
            TransactionEntry::Disputed { client, amount },
        );
        Ok(())
    }

    fn process_resolve(&mut self, transaction: &Transaction) -> Result<(), ProcessingError> {
        let (client, amount) = match self.entries.get(&transaction.transaction_id) {
            Some(TransactionEntry::Disputed { client, amount }) => (*client, *amount),
            _ => return Err(ProcessingError::UnknownTx),
        };
        if client != transaction.client {
            return Err(ProcessingError::NotOwner);
        }
        let account = self.accounts.entry(transaction.client).or_default();
        account.release(amount)?;
        self.entries.insert(
            transaction.transaction_id,
            TransactionEntry::Settled { client, amount },
        );
        Ok(())
    }

    fn process_chargeback(&mut self, transaction: &Transaction) -> Result<(), ProcessingError> {
        let (client, amount) = match self.entries.get(&transaction.transaction_id) {
            Some(TransactionEntry::Disputed { client, amount }) => (*client, *amount),
            _ => return Err(ProcessingError::UnknownTx),
        };
        if client != transaction.client {
            return Err(ProcessingError::NotOwner);
        }
        let account = self.accounts.entry(transaction.client).or_default();
        account.chargeback(amount)?;
        // Terminal: the charged-back record is removed, never re-stored.
        self.entries.remove(&transaction.transaction_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Amount;
    use crate::client::ClientId;
    use crate::transaction::{Transaction, TransactionId, TransactionType};

    fn deposit(client: ClientId, transaction_id: TransactionId, amount: &str) -> Transaction {
        Transaction {
            transaction_type: TransactionType::Deposit,
            client,
            transaction_id,
            amount: Some(Amount::parse(amount).unwrap()),
        }
    }

    fn withdrawal(client: ClientId, transaction_id: TransactionId, amount: &str) -> Transaction {
        Transaction {
            transaction_type: TransactionType::Withdrawal,
            client,
            transaction_id,
            amount: Some(Amount::parse(amount).unwrap()),
        }
    }

    fn dispute(client: ClientId, transaction_id: TransactionId) -> Transaction {
        Transaction {
            transaction_type: TransactionType::Dispute,
            client,
            transaction_id,
            amount: None,
        }
    }

    fn resolve(client: ClientId, transaction_id: TransactionId) -> Transaction {
        Transaction {
            transaction_type: TransactionType::Resolve,
            client,
            transaction_id,
            amount: None,
        }
    }

    fn chargeback(client: ClientId, transaction_id: TransactionId) -> Transaction {
        Transaction {
            transaction_type: TransactionType::Chargeback,
            client,
            transaction_id,
            amount: None,
        }
    }

    #[test]
    fn entries_deposits_and_withdrawals_in_index() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        processor.process(&withdrawal(1, 2, "25.00")).unwrap();
        assert!(matches!(
            processor.entries.get(&1),
            Some(TransactionEntry::Settled { .. })
        ));
        assert!(matches!(
            processor.entries.get(&2),
            Some(TransactionEntry::Settled { .. })
        ));
        assert_eq!(processor.entries[&1].client(), 1);
        assert_eq!(processor.entries[&1].amount(), Amount(1_002_500));
        assert_eq!(processor.entries[&2].client(), 1);
    }

    #[test]
    fn deposit_credits_available_and_total() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        let account = &processor.accounts[&1];
        assert_eq!(account.available, Amount(1_002_500));
        assert_eq!(account.held, Amount::ZERO);
        assert_eq!(account.total(), Amount(1_002_500));
    }

    #[test]
    fn sufficient_withdrawal_debits() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        processor.process(&withdrawal(1, 2, "25.00")).unwrap();
        let account = &processor.accounts[&1];
        assert_eq!(account.available, Amount(752_500));
        assert_eq!(account.total(), Amount(752_500));
    }

    #[test]
    fn insufficient_withdrawal_is_skipped_unchanged() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "50.00")).unwrap();
        assert!(processor.process(&withdrawal(1, 2, "75.00")).is_err());
        let account = &processor.accounts[&1];
        assert_eq!(account.available, Amount(500_000));
        assert_eq!(account.total(), Amount(500_000));
        assert!(!processor.entries.contains_key(&2));
    }

    #[test]
    fn dispute_holds_funds_and_keeps_total() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        processor.process(&dispute(1, 1)).unwrap();
        let account = &processor.accounts[&1];
        assert_eq!(account.available, Amount::ZERO);
        assert_eq!(account.held, Amount(1_002_500));
        assert_eq!(account.total(), Amount(1_002_500));
        assert!(matches!(
            processor.entries.get(&1),
            Some(TransactionEntry::Disputed { .. })
        ));
        assert!(!matches!(
            processor.entries.get(&1),
            Some(TransactionEntry::Settled { .. })
        ));
    }

    #[test]
    fn dispute_can_drive_available_negative() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.00")).unwrap();
        processor.process(&withdrawal(1, 2, "80.00")).unwrap();
        processor.process(&dispute(1, 1)).unwrap();

        let account = &processor.accounts[&1];
        assert_eq!(account.available, Amount(-800_000));
        assert_eq!(account.held, Amount(1_000_000));
        assert_eq!(account.total(), Amount(200_000));
    }

    #[test]
    fn dispute_of_unknown_transaction_is_skipped() {
        let mut processor = TransactionProcessor::new();
        assert!(processor.process(&dispute(1, 99)).is_err());
    }

    #[test]
    fn dispute_of_already_disputed_transaction_is_skipped() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        processor.process(&dispute(1, 1)).unwrap();
        assert!(processor.process(&dispute(1, 1)).is_err());
    }

    #[test]
    fn resolve_releases_held_funds() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        processor.process(&dispute(1, 1)).unwrap();
        processor.process(&resolve(1, 1)).unwrap();
        let account = &processor.accounts[&1];
        assert_eq!(account.held, Amount::ZERO);
        assert_eq!(account.available, Amount(1_002_500));
        assert_eq!(account.total(), Amount(1_002_500));
        assert!(matches!(
            processor.entries.get(&1),
            Some(TransactionEntry::Settled { .. })
        ));
        assert!(!matches!(
            processor.entries.get(&1),
            Some(TransactionEntry::Disputed { .. })
        ));
    }

    #[test]
    fn resolve_of_non_disputed_transaction_is_skipped() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        assert!(processor.process(&resolve(1, 1)).is_err());
    }

    #[test]
    fn chargeback_withdraws_held_and_locks_account() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        processor.process(&dispute(1, 1)).unwrap();
        processor.process(&chargeback(1, 1)).unwrap();
        let account = &processor.accounts[&1];
        assert_eq!(account.held, Amount::ZERO);
        assert_eq!(account.available, Amount::ZERO);
        assert_eq!(account.total(), Amount::ZERO);
        assert!(account.locked);
        assert!(!processor.entries.contains_key(&1));
    }

    #[test]
    fn chargeback_of_non_disputed_transaction_is_skipped() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        assert!(processor.process(&chargeback(1, 1)).is_err());
        assert!(!processor.accounts[&1].locked);
    }

    #[test]
    fn locked_account_rejects_all_operations() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        processor.process(&dispute(1, 1)).unwrap();
        processor.process(&chargeback(1, 1)).unwrap();
        assert!(processor.accounts[&1].locked);
        assert!(processor.process(&deposit(1, 2, "50.00")).is_err());
        assert!(processor.process(&withdrawal(1, 3, "1.00")).is_err());
        let account = &processor.accounts[&1];
        assert_eq!(account.total(), Amount::ZERO);
    }

    #[test]
    fn cross_client_dispute_is_skipped() {
        let mut processor = TransactionProcessor::new();
        processor.process(&deposit(1, 1, "100.25")).unwrap();
        assert!(processor.process(&dispute(2, 1)).is_err());
        let account = &processor.accounts[&1];
        assert_eq!(account.available, Amount(1_002_500));
        assert_eq!(account.held, Amount::ZERO);
    }

    #[test]
    fn unknown_client_is_initialized_zeroed() {
        let mut processor = TransactionProcessor::new();
        assert!(processor.process(&withdrawal(7, 1, "10.00")).is_err());
        assert!(processor.accounts.contains_key(&7));
        assert_eq!(processor.accounts[&7].total(), Amount::ZERO);
    }
}
