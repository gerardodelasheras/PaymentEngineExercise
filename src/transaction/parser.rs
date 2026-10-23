//! Transaction CSV parsing: raw row deserialization and domain validation.

use std::io::Read;

use csv::{ReaderBuilder, StringRecord, Trim};
use serde::Deserialize;

use crate::amount::Amount;
use crate::client::client_id::ClientId;
use crate::processing_error::ProcessingError;
use crate::transaction::transaction::Transaction;
use crate::transaction::transaction_id::TransactionId;
use crate::transaction::transaction_type::TransactionType;

const REQUIRED_COLUMNS: [&str; 4] = ["type", "client", "tx", "amount"];

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
        let raw_type = self.transaction_type.trim();
        let transaction_type = match raw_type {
            "deposit" => TransactionType::Deposit,
            "withdrawal" => TransactionType::Withdrawal,
            "dispute" => TransactionType::Dispute,
            "resolve" => TransactionType::Resolve,
            "chargeback" => TransactionType::Chargeback,
            _ => return Err(ProcessingError::InvalidType(raw_type.to_string())),
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
/// A malformed header is an [`ProcessingError::InvalidHeader`] error returned by
/// this function. A row that fails deserialization surfaces as
/// [`ProcessingError::Csv`]; a row that fails domain validation surfaces as its
/// specific variant.
pub fn transactions<R: Read>(
    reader: R,
) -> Result<impl Iterator<Item = Result<Transaction, ProcessingError>>, ProcessingError> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .trim(Trim::All)
        .from_reader(reader);

    let headers = reader.headers().map_err(ProcessingError::Csv)?.clone();
    validate_header(&headers)?;

    Ok(Rows {
        reader,
        headers,
        record: StringRecord::new(),
    })
}

struct Rows<R: Read> {
    reader: csv::Reader<R>,
    headers: StringRecord,
    record: StringRecord,
}

impl<R: Read> Iterator for Rows<R> {
    type Item = Result<Transaction, ProcessingError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.record.clear();
        match self.reader.read_record(&mut self.record) {
            Ok(false) => None,
            Ok(true) => {
                // A short non-monetary row omits the trailing amount field. Pad it
                // to the header width so serde can apply `#[serde(default)]`.
                while self.record.len() < self.headers.len() {
                    self.record.push_field("");
                }
                Some(
                    self.record
                        .deserialize::<RawTransactionRow>(Some(&self.headers))
                        .map_err(ProcessingError::Csv)
                        .and_then(RawTransactionRow::into_transaction),
                )
            }
            Err(error) => Some(Err(ProcessingError::Csv(error))),
        }
    }
}

fn validate_header(headers: &StringRecord) -> Result<(), ProcessingError> {
    for required in REQUIRED_COLUMNS {
        if !headers.iter().any(|name| name == required) {
            return Err(ProcessingError::InvalidHeader(format!(
                "missing required column '{required}'"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse_all(input: &str) -> Vec<Result<Transaction, ProcessingError>> {
        transactions(Cursor::new(input.to_string()))
            .expect("header is valid")
            .collect()
    }

    const HEADER: &str = "type,client,tx,amount\n";

    #[test]
    fn parses_a_deposit() {
        let rows = parse_all(&format!("{HEADER}deposit,1,1,2.0\n"));
        let transaction = rows[0].as_ref().expect("valid row");
        assert_eq!(transaction.transaction_type, TransactionType::Deposit);
        assert_eq!(transaction.client, 1);
        assert_eq!(transaction.transaction_id, 1);
        assert_eq!(transaction.amount.unwrap().to_string(), "2.0");
    }

    #[test]
    fn trims_padded_fields() {
        let rows = parse_all(&format!("{HEADER} deposit , 1 , 2 , 3.0 \n"));
        let transaction = rows[0].as_ref().expect("valid row");
        assert_eq!(transaction.transaction_type, TransactionType::Deposit);
        assert_eq!(transaction.client, 1);
        assert_eq!(transaction.amount.unwrap().to_string(), "3.0");
    }

    #[test]
    fn accepts_a_dispute_without_an_amount_column() {
        let rows = parse_all(&format!("{HEADER}deposit,1,1,10.0\ndispute,1,1\n"));
        assert!(rows[1].is_ok());
        assert_eq!(rows[1].as_ref().unwrap().amount, None);
    }

    #[test]
    fn accepts_a_dispute_with_an_empty_amount_field() {
        let rows = parse_all(&format!("{HEADER}deposit,1,1,10.0\ndispute,1,1,\n"));
        assert!(rows[1].is_ok());
    }

    #[test]
    fn rejects_a_dispute_with_an_amount() {
        let rows = parse_all(&format!("{HEADER}deposit,1,1,10.0\ndispute,1,1,5.0\n"));
        assert!(matches!(rows[1], Err(ProcessingError::UnexpectedAmount)));
    }

    #[test]
    fn rejects_a_monetary_row_without_an_amount() {
        let rows = parse_all(&format!("{HEADER}deposit,1,1,\n"));
        assert!(matches!(rows[0], Err(ProcessingError::MissingAmount)));
    }

    #[test]
    fn rejects_unknown_types() {
        let rows = parse_all(&format!("{HEADER}transfer,1,1,5.0\n"));
        assert!(matches!(rows[0], Err(ProcessingError::InvalidType(_))));
    }

    #[test]
    fn rejects_clients_outside_u16() {
        let rows = parse_all(&format!("{HEADER}deposit,70000,1,5.0\n"));
        assert!(matches!(rows[0], Err(ProcessingError::Csv(_))));
    }

    #[test]
    fn rejects_transactions_outside_u32() {
        let rows = parse_all(&format!("{HEADER}deposit,1,5000000000,5.0\n"));
        assert!(matches!(rows[0], Err(ProcessingError::Csv(_))));
    }

    #[test]
    fn rejects_rows_with_too_few_columns() {
        let rows = parse_all(&format!("{HEADER}deposit,1\n"));
        assert!(matches!(rows[0], Err(ProcessingError::Csv(_))));
    }

    #[test]
    fn ignores_blank_lines() {
        let rows = parse_all(&format!("{HEADER}deposit,1,1,3.0\n\ndeposit,1,2,4.0\n"));
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.is_ok()));
    }

    #[test]
    fn rejects_an_invalid_header() {
        let error = transactions(Cursor::new("kind,client,tx,amount\n".to_string()))
            .err()
            .expect("header is rejected");
        assert!(matches!(error, ProcessingError::InvalidHeader(_)));
    }

    #[test]
    fn accepts_reordered_header_columns() {
        let rows = parse_all("tx,amount,type,client\n3,2.0,deposit,1\n");
        let transaction = rows[0].as_ref().expect("valid row");
        assert_eq!(transaction.transaction_type, TransactionType::Deposit);
        assert_eq!(transaction.client, 1);
        assert_eq!(transaction.transaction_id, 3);
        assert_eq!(transaction.amount.unwrap().to_string(), "2.0");
    }

    #[test]
    fn ignores_extra_columns() {
        let rows = parse_all("type,client,tx,amount,note\ndeposit,1,1,2.0,ok\n");
        assert!(rows[0].is_ok());
    }

    #[test]
    fn rejects_a_missing_required_column() {
        let error = transactions(Cursor::new(
            "type,client,amount\ndeposit,1,5.0\n".to_string(),
        ))
        .err()
        .expect("header is rejected");
        assert!(matches!(error, ProcessingError::InvalidHeader(_)));
    }

    #[test]
    fn rejects_an_uppercase_header() {
        let error = transactions(Cursor::new(
            "TYPE,CLIENT,TX,AMOUNT\ndeposit,1,1,2.0\n".to_string(),
        ))
        .err()
        .expect("header is rejected");
        assert!(matches!(error, ProcessingError::InvalidHeader(_)));
    }

    #[test]
    fn invalid_utf8_rows_do_not_stop_the_stream() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(HEADER.as_bytes());
        bytes.extend_from_slice(b"deposit,1,1,5.0\n");
        bytes.extend_from_slice(b"deposit,1,2,\xff\n");
        bytes.extend_from_slice(b"deposit,1,3,6.0\n");

        let rows: Vec<_> = transactions(Cursor::new(bytes))
            .expect("header is valid")
            .collect();
        assert!(rows[0].is_ok());
        assert!(rows[1].is_err());
        assert!(rows[2].is_ok());
    }
}
