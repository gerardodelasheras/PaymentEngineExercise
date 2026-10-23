//! Shared core for the streaming transaction processor.
//!
//! All domain logic, error handling, and account actor loops live here so the
//! standard `transaction_processor` binary can reuse the same processing core.

pub mod account;
pub mod amount;
pub mod client;
pub mod pipeline;
pub mod processing_error;
pub mod transaction;

pub(crate) mod worker_pool;

pub use account::report as account_report;
pub use account::Account;
pub use amount::Amount;
pub use client::ClientId;
pub use processing_error::ProcessingError;
pub use transaction::{
    Transaction, TransactionEntry, TransactionId, TransactionProcessor, TransactionType,
};
