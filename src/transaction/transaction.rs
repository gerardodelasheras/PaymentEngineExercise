//! The parsed transaction input row.

use super::transaction_id::TransactionId;
use super::transaction_type::TransactionType;
use crate::amount::Amount;
use crate::client::ClientId;

/// A parsed transaction row.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub transaction_type: TransactionType,
    pub client: ClientId,
    pub transaction_id: TransactionId,
    /// Present only for `deposit` and `withdrawal`.
    pub amount: Option<Amount>,
}
