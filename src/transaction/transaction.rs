use crate::amount::Amount;
use crate::client::client_id::ClientId;
use crate::transaction::transaction_id::TransactionId;
use crate::transaction::transaction_type::TransactionType;

/// A parsed input row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub transaction_type: TransactionType,
    pub client: ClientId,
    pub transaction_id: TransactionId,
    pub amount: Option<Amount>,
}
