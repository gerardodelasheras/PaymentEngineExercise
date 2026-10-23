//! Recoverable errors for transaction processing.

use std::io;

/// Failures transaction processing can deliberately produce. Row-level variants are
/// non-fatal: the pipeline skips the offending row and continues.
/// Infrastructure variants such as [`ProcessingError::WorkerUnavailable`] fail the
/// run instead of silently omitting rows.
#[derive(Debug, thiserror::Error)]
pub enum ProcessingError {
    #[error("unknown transaction type")]
    InvalidType,
    #[error("invalid client id")]
    InvalidClient,
    #[error("invalid transaction id")]
    InvalidTx,
    #[error("invalid amount")]
    InvalidAmount,
    #[error("negative amount is not allowed")]
    NegativeAmount,
    #[error("amount has too many decimal places")]
    TooManyDecimals,
    #[error("amount is missing for a monetary transaction")]
    MissingAmount,
    #[error("unexpected amount on a non-monetary transaction")]
    UnexpectedAmount,
    #[error("account is locked")]
    AccountLocked,
    #[error("insufficient funds")]
    InsufficientFunds,
    #[error("referenced transaction does not exist")]
    UnknownTx,
    #[error("referenced transaction belongs to another client")]
    NotOwner,
    #[error("arithmetic overflow or underflow")]
    Overflow,
    #[error("worker or producer channel closed before all rows were processed")]
    WorkerUnavailable,
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),
}
