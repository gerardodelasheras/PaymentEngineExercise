use crate::processing_error::ProcessingError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionType {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

impl TransactionType {
    pub fn parse(input: &str) -> Result<Self, ProcessingError> {
        match input.trim() {
            "deposit" => Ok(TransactionType::Deposit),
            "withdrawal" => Ok(TransactionType::Withdrawal),
            "dispute" => Ok(TransactionType::Dispute),
            "resolve" => Ok(TransactionType::Resolve),
            "chargeback" => Ok(TransactionType::Chargeback),
            other => Err(ProcessingError::InvalidType(other.to_string())),
        }
    }

    pub fn is_monetary(self) -> bool {
        matches!(self, TransactionType::Deposit | TransactionType::Withdrawal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_supported_type() {
        assert_eq!(
            TransactionType::parse("deposit").unwrap(),
            TransactionType::Deposit
        );
        assert_eq!(
            TransactionType::parse("withdrawal").unwrap(),
            TransactionType::Withdrawal
        );
        assert_eq!(
            TransactionType::parse("dispute").unwrap(),
            TransactionType::Dispute
        );
        assert_eq!(
            TransactionType::parse("resolve").unwrap(),
            TransactionType::Resolve
        );
        assert_eq!(
            TransactionType::parse("chargeback").unwrap(),
            TransactionType::Chargeback
        );
    }

    #[test]
    fn trims_type_text() {
        assert_eq!(
            TransactionType::parse(" deposit ").unwrap(),
            TransactionType::Deposit
        );
    }

    #[test]
    fn rejects_unknown_types() {
        assert!(matches!(
            TransactionType::parse("transfer"),
            Err(ProcessingError::InvalidType(_))
        ));
    }
}
