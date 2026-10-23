//! Client account state.

use crate::amount::Amount;
use crate::processing_error::ProcessingError;

/// A single client's ledger profile.
#[derive(Debug, Clone, Default)]
pub struct Account {
    /// Funds available for withdrawal. Equal to `total - held`.
    pub available: Amount,
    /// Funds frozen by an active dispute. Equal to `total - available`.
    pub held: Amount,
    /// True once a chargeback has occurred; the account is then frozen.
    pub locked: bool,
}

impl Account {
    /// Total funds, always equal to `available + held`.
    pub fn total(&self) -> Amount {
        self.available + self.held
    }

    /// Credit `amount` to `available` (and therefore `total`).
    pub fn credit(&mut self, amount: Amount) -> Result<(), ProcessingError> {
        self.available = self
            .available
            .checked_add(amount)
            .ok_or(ProcessingError::Overflow)?;
        Ok(())
    }

    /// Debit `amount` from `available`, failing when funds are insufficient.
    pub fn debit(&mut self, amount: Amount) -> Result<(), ProcessingError> {
        if self.available < amount {
            return Err(ProcessingError::InsufficientFunds);
        }
        self.available = self
            .available
            .checked_sub(amount)
            .ok_or(ProcessingError::Overflow)?;
        Ok(())
    }

    /// Move `amount` from `available` to `held` for an active dispute.
    pub fn hold(&mut self, amount: Amount) -> Result<(), ProcessingError> {
        let available = self
            .available
            .checked_sub(amount)
            .ok_or(ProcessingError::Overflow)?;
        let held = self
            .held
            .checked_add(amount)
            .ok_or(ProcessingError::Overflow)?;
        self.available = available;
        self.held = held;
        Ok(())
    }

    /// Move `amount` from `held` back to `available` on dispute resolution.
    pub fn release(&mut self, amount: Amount) -> Result<(), ProcessingError> {
        let held = self
            .held
            .checked_sub(amount)
            .ok_or(ProcessingError::Overflow)?;
        let available = self
            .available
            .checked_add(amount)
            .ok_or(ProcessingError::Overflow)?;
        self.held = held;
        self.available = available;
        Ok(())
    }

    /// Withdraw `amount` from `held` and freeze the account.
    pub fn chargeback(&mut self, amount: Amount) -> Result<(), ProcessingError> {
        self.held = self
            .held
            .checked_sub(amount)
            .ok_or(ProcessingError::Overflow)?;
        self.locked = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zeroed_and_unlocked() {
        let account = Account::default();
        assert_eq!(account.available, Amount::ZERO);
        assert_eq!(account.held, Amount::ZERO);
        assert_eq!(account.total(), Amount::ZERO);
        assert!(!account.locked);
    }

    #[test]
    fn total_is_available_plus_held() {
        let account = Account {
            available: Amount(250_000), // 25.0
            held: Amount(120_000),      // 12.0
            locked: false,
        };
        assert_eq!(account.total(), Amount(370_000));
    }

    #[test]
    fn hold_overflow_leaves_balances_unchanged() {
        let mut account = Account {
            available: Amount(0),
            held: Amount(i64::MAX),
            locked: false,
        };

        let result = account.hold(Amount(1));

        assert!(matches!(result, Err(ProcessingError::Overflow)));
        assert_eq!(account.available, Amount(0));
        assert_eq!(account.held, Amount(i64::MAX));
    }

    #[test]
    fn release_overflow_leaves_balances_unchanged() {
        let mut account = Account {
            available: Amount(i64::MAX),
            held: Amount(1),
            locked: false,
        };

        let result = account.release(Amount(1));

        assert!(matches!(result, Err(ProcessingError::Overflow)));
        assert_eq!(account.available, Amount(i64::MAX));
        assert_eq!(account.held, Amount(1));
    }
}
