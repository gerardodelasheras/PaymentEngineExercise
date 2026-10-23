//! The transaction model, input parsing, dispute lifecycle, and per-client
//! processor.
//!
//! `TransactionProcessor` owns the accounts and transaction index for the
//! clients mapped to a single worker actor. The dispute lifecycle is
//! encoded in the transaction entry's variants (see `transaction_entry`).

pub mod parser;
#[allow(clippy::module_inception)]
mod transaction;
mod transaction_entry;
mod transaction_id;
mod transaction_processor;
mod transaction_type;

pub use transaction::Transaction;
pub use transaction_entry::TransactionEntry;
pub use transaction_id::TransactionId;
pub use transaction_processor::TransactionProcessor;
pub use transaction_type::TransactionType;
