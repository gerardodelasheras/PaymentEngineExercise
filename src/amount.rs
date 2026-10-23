//! Fixed-point currency: every amount is an `i64` scaled by 10_000.

use crate::processing_error::ProcessingError;

/// The fixed scale factor: `amount * 10_000` (e.g. `1.23` -> `12300`).
pub const SCALE: i64 = 10_000;

/// An amount expressed as a scaled `i64`. Floating point is never used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Amount(pub i64);

impl Amount {
    /// The zero amount.
    pub const ZERO: Amount = Amount(0);

    /// Parse a decimal string into a scaled `Amount` without any floating
    /// point. Accepts up to four fraction digits and a leading decimal point
    /// (`.5` == `0.5`); rejects negatives, empty strings, a bare `.`, and
    /// more-than-four-digit fractions.
    pub fn parse(s: &str) -> Result<Amount, ProcessingError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ProcessingError::InvalidAmount);
        }
        let (int_str, frac_str) = match s.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (s, None),
        };
        if int_str.starts_with('-') {
            return Err(ProcessingError::NegativeAmount);
        }
        let int_val: i64 = if int_str.is_empty() {
            // A leading decimal point is allowed (".5" == "0.5"), but a bare
            // "." carries no digits at all.
            match frac_str {
                Some(f) if !f.is_empty() => 0,
                _ => return Err(ProcessingError::InvalidAmount),
            }
        } else {
            int_str
                .parse()
                .map_err(|_| ProcessingError::InvalidAmount)?
        };
        let mut value = int_val
            .checked_mul(SCALE)
            .ok_or(ProcessingError::Overflow)?;

        if let Some(frac) = frac_str {
            if frac.len() > 4 {
                return Err(ProcessingError::TooManyDecimals);
            }
            if !frac.is_empty() {
                if !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ProcessingError::InvalidAmount);
                }
                let frac_val: i64 = frac.parse().map_err(|_| ProcessingError::InvalidAmount)?;
                let multiplier = 10_i64.pow((4 - frac.len()) as u32);
                value = value
                    .checked_add(
                        frac_val
                            .checked_mul(multiplier)
                            .ok_or(ProcessingError::Overflow)?,
                    )
                    .ok_or(ProcessingError::Overflow)?;
            }
        }
        Ok(Amount(value))
    }

    /// Saturating-free checked addition.
    pub fn checked_add(self, other: Amount) -> Option<Amount> {
        self.0.checked_add(other.0).map(Amount)
    }

    /// Saturating-free checked subtraction.
    pub fn checked_sub(self, other: Amount) -> Option<Amount> {
        self.0.checked_sub(other.0).map(Amount)
    }
}

impl std::fmt::Display for Amount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = self.0;
        let negative = value < 0;
        let value = value.unsigned_abs();
        let integer = value / SCALE as u64;
        let mut fraction = value % SCALE as u64;

        if fraction == 0 {
            return if negative {
                write!(formatter, "-{integer}.0")
            } else {
                write!(formatter, "{integer}.0")
            };
        }

        let mut digits = 4;
        while digits > 0 && fraction.is_multiple_of(10) {
            fraction /= 10;
            digits -= 1;
        }

        if negative {
            write!(formatter, "-{integer}.{fraction:0width$}", width = digits)
        } else {
            write!(formatter, "{integer}.{fraction:0width$}", width = digits)
        }
    }
}

impl std::ops::Add for Amount {
    type Output = Amount;

    fn add(self, rhs: Amount) -> Amount {
        Amount(
            self.0
                .checked_add(rhs.0)
                .expect("amount addition overflow"),
        )
    }
}

impl std::ops::Sub for Amount {
    type Output = Amount;

    fn sub(self, rhs: Amount) -> Amount {
        Amount(
            self.0
                .checked_sub(rhs.0)
                .expect("amount subtraction overflow"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whole_numbers() {
        assert_eq!(Amount::parse("42").unwrap(), Amount(420_000));
        assert_eq!(Amount::parse("0").unwrap(), Amount(0));
    }

    #[test]
    fn parses_varied_fraction_lengths() {
        assert_eq!(Amount::parse("3.14").unwrap(), Amount(31_400));
        assert_eq!(Amount::parse("0.5").unwrap(), Amount(5_000));
        assert_eq!(Amount::parse("123.4567").unwrap(), Amount(1_234_567));
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(Amount::parse("  9.99  ").unwrap(), Amount(99_900));
    }

    #[test]
    fn rejects_negative_amounts() {
        assert!(Amount::parse("-1.0").is_err());
        assert!(Amount::parse("-0.25").is_err());
    }

    #[test]
    fn rejects_more_than_four_decimals() {
        assert!(Amount::parse("1.00001").is_err());
    }

    #[test]
    fn rejects_empty_and_malformed() {
        assert!(Amount::parse("").is_err());
        assert!(Amount::parse(".").is_err());
        assert!(Amount::parse("1.2.3").is_err());
        assert!(Amount::parse("abc").is_err());
    }

    #[test]
    fn accepts_fraction_without_leading_zero() {
        assert_eq!(Amount::parse(".5").unwrap(), Amount(5_000));
        assert_eq!(Amount::parse(".25").unwrap(), Amount(2_500));
        assert_eq!(Amount::parse(".0001").unwrap(), Amount(1));
    }

    #[test]
    fn rejects_amounts_that_overflow_i64() {
        // i64::MAX scaled by 10_000 overflows the fixed-point representation.
        assert!(Amount::parse("9223372036854775807").is_err());
        // A value larger than i64 itself fails to parse.
        assert!(Amount::parse("99999999999999999999").is_err());
    }

    #[test]
    fn checked_arithmetic_detects_overflow() {
        assert_eq!(Amount(100).checked_add(Amount(50)), Some(Amount(150)));
        assert_eq!(Amount(100).checked_sub(Amount(50)), Some(Amount(50)));
        assert_eq!(Amount(i64::MAX).checked_add(Amount(1)), None);
        assert_eq!(Amount(i64::MIN).checked_sub(Amount(1)), None);
    }

    #[test]
    fn formats_with_trailing_zeros_stripped() {
        assert_eq!(Amount(150_000).to_string(), "15.0");
        assert_eq!(Amount(31_400).to_string(), "3.14");
        assert_eq!(Amount(5_000).to_string(), "0.5");
        assert_eq!(Amount(0).to_string(), "0.0");
        assert_eq!(Amount(1_234_567).to_string(), "123.4567");
        assert_eq!(Amount(-5_000).to_string(), "-0.5");
        assert_eq!(Amount(-1).to_string(), "-0.0001");
    }

    #[test]
    fn operators_add_and_subtract() {
        assert_eq!(Amount(100) + Amount(50), Amount(150));
        assert_eq!(Amount(100) - Amount(50), Amount(50));
    }

    #[test]
    #[should_panic(expected = "amount addition overflow")]
    fn operator_addition_panics_on_overflow() {
        let _ = Amount(i64::MAX) + Amount(1);
    }

    #[test]
    #[should_panic(expected = "amount subtraction overflow")]
    fn operator_subtraction_panics_on_overflow() {
        let _ = Amount(i64::MIN) - Amount(1);
    }
}
