//! Transaction CSV parsing: raw row deserialization and domain validation.

use std::io::Read;

use serde::Deserialize;

use super::{Transaction, TransactionId, TransactionType};
use crate::amount::Amount;
use crate::client::ClientId;
use crate::processing_error::ProcessingError;

/// A raw CSV record before domain validation.
#[derive(Debug, Deserialize)]
struct RawTransactionRow {
    #[serde(rename = "type")]
    transaction_type: String,
    client: ClientId,
    #[serde(rename = "tx")]
    transaction_id: TransactionId,
    #[serde(default)]
    amount: String,
}

impl RawTransactionRow {
    fn into_transaction(self) -> Result<Transaction, ProcessingError> {
        let transaction_type = match self.transaction_type.trim() {
            "deposit" => TransactionType::Deposit,
            "withdrawal" => TransactionType::Withdrawal,
            "dispute" => TransactionType::Dispute,
            "resolve" => TransactionType::Resolve,
            "chargeback" => TransactionType::Chargeback,
            _ => return Err(ProcessingError::InvalidType),
        };
        let amount_field = self.amount.trim();
        let amount = match transaction_type {
            TransactionType::Deposit | TransactionType::Withdrawal => {
                if amount_field.is_empty() {
                    return Err(ProcessingError::MissingAmount);
                }
                Some(Amount::parse(amount_field)?)
            }
            _ => {
                if !amount_field.is_empty() {
                    return Err(ProcessingError::UnexpectedAmount);
                }
                None
            }
        };
        Ok(Transaction {
            transaction_type,
            client: self.client,
            transaction_id: self.transaction_id,
            amount,
        })
    }
}

/// Stream `reader` as validated [`Transaction`]s, one `Result` per row.
///
/// A malformed CSV record surfaces as [`ProcessingError::Csv`]; a row that fails
/// domain validation surfaces as its specific [`ProcessingError`] variant. The
/// caller decides how to handle each failure (typically skip and log).
pub fn transactions<R: Read>(
    reader: R,
) -> impl Iterator<Item = Result<Transaction, ProcessingError>> {
    csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .has_headers(true)
        .from_reader(reader)
        .into_deserialize::<RawTransactionRow>()
        .map(|result| result?.into_transaction())
}
