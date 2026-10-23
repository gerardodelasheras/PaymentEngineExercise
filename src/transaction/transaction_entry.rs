//! The transaction dispute lifecycle as a single enum.
//!
//! A transaction entry's dispute state is carried in its variant, so a
//! transition is a `match` on the stored value: `dispute` moves `Settled` to
//! `Disputed`, `resolve` moves `Disputed` back to `Settled`, and `chargeback`
//! removes the entry (terminal). Wrong-state references are rejected by the
//! processor at runtime.

use crate::amount::Amount;
use crate::client::ClientId;

/// An accepted deposit or withdrawal entry and its dispute lifecycle.
#[derive(Debug, Clone)]
pub enum TransactionEntry {
    /// Settled and eligible to be disputed.
    Settled { client: ClientId, amount: Amount },
    /// Currently under active dispute.
    Disputed { client: ClientId, amount: Amount },
}

impl TransactionEntry {
    /// The client that owns this transaction entry.
    pub fn client(&self) -> ClientId {
        match self {
            TransactionEntry::Settled { client, .. }
            | TransactionEntry::Disputed { client, .. } => *client,
        }
    }

    /// The transaction entry's scaled amount.
    pub fn amount(&self) -> Amount {
        match self {
            TransactionEntry::Settled { amount, .. }
            | TransactionEntry::Disputed { amount, .. } => *amount,
        }
    }
}
