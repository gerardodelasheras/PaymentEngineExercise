use crate::amount::Amount;
use crate::client::client_id::ClientId;

/// An accepted monetary transaction kept for later dispute operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionEntry {
    Settled { client: ClientId, amount: Amount },
    Disputed { client: ClientId, amount: Amount },
}
