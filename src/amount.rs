//! Fixed-point currency: every amount is an `i64` scaled by 10_000.

use std::fmt;
use std::ops::{Add, Sub};

use crate::processing_error::ProcessingError;

/// Number of fixed-point units in one whole currency unit.
///
/// For example, `1.23` is stored as `12_300` (`1.23 * 10_000`).
pub const SCALE: i64 = 10_000;

/// A monetary amount stored as an `i64` scaled by [`SCALE`].
///
/// Floating point is never used for parsing, arithmetic, or rendering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(i64);

impl Amount {
    /// The zero amount.
    pub const ZERO: Amount = Amount(0);

    /// Creates an amount from an already scaled integer.
    ///
    /// This bypasses decimal parsing and is intended for tests and boundary
    /// values.
    pub fn from_scaled(scaled: i64) -> Self {
        Amount(scaled)
    }

    /// Returns the underlying scaled integer.
    pub fn scaled(self) -> i64 {
        self.0
    }

    /// Parses a non-negative decimal string with at most four fractional digits.
    ///
    /// Accepted forms include whole values (`"42"`), fractional values
    /// (`"1.23"`, `"1.2300"`), a leading decimal point (`".5"`), and
    /// surrounding whitespace.
    ///
    /// Rejected forms include empty input, a bare `"."`, more than one decimal
    /// point, non-digit text, a leading sign, fractions longer than four
    /// digits, and values that do not fit in the scaled `i64`.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessingError::InvalidAmount`] for malformed text,
    /// [`ProcessingError::NegativeAmount`] for a leading `-`,
    /// [`ProcessingError::TooManyDecimals`] for more than four fraction digits,
    /// and [`ProcessingError::Overflow`] when the scaled value does not fit.
    pub fn parse(input: &str) -> Result<Self, ProcessingError> {
        let text = input.trim();
        if text.is_empty() {
            return Err(ProcessingError::InvalidAmount(input.to_string()));
        }
        if text.starts_with('-') {
            return Err(ProcessingError::NegativeAmount(input.to_string()));
        }
        if text.starts_with('+') {
            return Err(ProcessingError::InvalidAmount(input.to_string()));
        }

        // Split once and reject a second decimal point before validating digits.
        let mut parts = text.split('.');
        let integer = parts.next().unwrap_or("");
        let fraction = parts.next();
        if parts.next().is_some() {
            return Err(ProcessingError::InvalidAmount(input.to_string()));
        }
        let fraction = fraction.unwrap_or("");

        // "." and "" carry no digits at all.
        if integer.is_empty() && fraction.is_empty() {
            return Err(ProcessingError::InvalidAmount(input.to_string()));
        }
        if !integer.chars().all(|c| c.is_ascii_digit()) {
            return Err(ProcessingError::InvalidAmount(input.to_string()));
        }
        if !fraction.chars().all(|c| c.is_ascii_digit()) {
            return Err(ProcessingError::InvalidAmount(input.to_string()));
        }
        if fraction.len() > 4 {
            return Err(ProcessingError::TooManyDecimals(input.to_string()));
        }

        let whole: i64 = if integer.is_empty() {
            0
        } else {
            integer.parse().map_err(|_| ProcessingError::Overflow)?
        };

        let fractional: i64 = if fraction.is_empty() {
            0
        } else {
            let fraction_value: i64 = fraction.parse().map_err(|_| ProcessingError::Overflow)?;
            let multiplier = 10_i64.pow((4 - fraction.len()) as u32);
            fraction_value
                .checked_mul(multiplier)
                .ok_or(ProcessingError::Overflow)?
        };

        let scaled = whole
            .checked_mul(SCALE)
            .and_then(|value| value.checked_add(fractional))
            .ok_or(ProcessingError::Overflow)?;
        Ok(Amount(scaled))
    }

    /// Returns `None` when the scaled sum would overflow `i64`.
    pub fn checked_add(self, other: Amount) -> Option<Amount> {
        self.0.checked_add(other.0).map(Amount)
    }

    /// Returns `None` when the scaled difference would overflow `i64`.
    pub fn checked_sub(self, other: Amount) -> Option<Amount> {
        self.0.checked_sub(other.0).map(Amount)
    }
}

/// Renders the scaled value with a decimal point, preserving the sign and
/// stripping trailing fractional zeros while keeping at least one fraction digit.
impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.0;
        let negative = value < 0;
        let magnitude = value.unsigned_abs();
        let whole = magnitude / SCALE as u64;
        let mut fraction = magnitude % SCALE as u64;
        let sign = if negative { "-" } else { "" };

        if fraction == 0 {
            return write!(f, "{sign}{whole}.0");
        }

        // Strip trailing zeros before zero-padding the remaining fraction width,
        // so 10 renders as "0.001" rather than "0.010".
        let mut digits = 4usize;
        while digits > 1 && fraction.is_multiple_of(10) {
            fraction /= 10;
            digits -= 1;
        }

        write!(f, "{sign}{whole}.{fraction:0width$}", width = digits)
    }
}

impl Add for Amount {
    type Output = Amount;

    /// # Panics
    ///
    /// Panics if the scaled sum overflows `i64`; use [`Amount::checked_add`]
    /// when the result must be handled.
    fn add(self, rhs: Amount) -> Amount {
        self.checked_add(rhs).expect("amount overflow")
    }
}

impl Sub for Amount {
    type Output = Amount;

    /// # Panics
    ///
    /// Panics if the scaled difference overflows `i64`; use
    /// [`Amount::checked_sub`] when the result must be handled.
    fn sub(self, rhs: Amount) -> Amount {
        self.checked_sub(rhs).expect("amount underflow")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(input: &str) -> Amount {
        Amount::parse(input).expect("amount should parse")
    }

    #[test]
    fn parses_whole_amounts() {
        assert_eq!(parsed("5").scaled(), 50_000);
        assert_eq!(parsed("5").to_string(), "5.0");
    }

    #[test]
    fn parses_fractional_amounts() {
        assert_eq!(parsed("1.23").scaled(), 12_300);
        assert_eq!(parsed("1.23").to_string(), "1.23");
    }

    #[test]
    fn parses_leading_dot_fractions() {
        assert_eq!(parsed(".5").scaled(), 5_000);
        assert_eq!(parsed(".5").to_string(), "0.5");
    }

    #[test]
    fn parses_trailing_fractional_zeros() {
        assert_eq!(parsed("1.2300").scaled(), 12_300);
        assert_eq!(parsed("1.2300").to_string(), "1.23");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(parsed("  1.5  ").scaled(), 15_000);
    }

    #[test]
    fn rejects_negative_amounts() {
        assert!(matches!(
            Amount::parse("-1.0"),
            Err(ProcessingError::NegativeAmount(_))
        ));
    }

    #[test]
    fn rejects_too_many_fraction_digits() {
        assert!(matches!(
            Amount::parse("1.23456"),
            Err(ProcessingError::TooManyDecimals(_))
        ));
    }

    #[test]
    fn rejects_malformed_amounts() {
        assert!(matches!(
            Amount::parse("abc"),
            Err(ProcessingError::InvalidAmount(_))
        ));
        assert!(matches!(
            Amount::parse("1.2.3"),
            Err(ProcessingError::InvalidAmount(_))
        ));
        assert!(matches!(
            Amount::parse(""),
            Err(ProcessingError::InvalidAmount(_))
        ));
    }

    #[test]
    fn rejects_scaled_overflow() {
        assert!(matches!(
            Amount::parse("922337203685477.5808"),
            Err(ProcessingError::Overflow)
        ));
    }

    #[test]
    fn accepts_the_largest_scaled_value() {
        assert_eq!(parsed("922337203685477.5807").scaled(), i64::MAX);
    }

    #[test]
    fn renders_zero_with_one_decimal_place() {
        assert_eq!(Amount::ZERO.to_string(), "0.0");
    }

    #[test]
    fn renders_one_unit_fractions() {
        assert_eq!(parsed("0.0001").to_string(), "0.0001");
    }

    #[test]
    fn renders_negative_amounts_with_a_sign() {
        let zero = Amount::ZERO;
        let value = zero.checked_sub(parsed("10.0")).expect("subtraction fits");
        assert_eq!(value.to_string(), "-10.0");
    }

    #[test]
    fn checked_add_reports_overflow() {
        assert!(Amount::from_scaled(i64::MAX)
            .checked_add(Amount::from_scaled(1))
            .is_none());
    }

    #[test]
    fn checked_sub_reports_underflow() {
        assert!(Amount::from_scaled(i64::MIN)
            .checked_sub(Amount::from_scaled(1))
            .is_none());
    }

    #[test]
    fn parses_leading_dot_fraction_lengths() {
        assert_eq!(parsed(".25").scaled(), 2_500);
        assert_eq!(parsed(".0001").scaled(), 1);
    }

    #[test]
    fn parses_supported_edge_forms() {
        assert_eq!(parsed("1.").scaled(), 10_000);
        assert_eq!(parsed("00.5").scaled(), 5_000);
        assert_eq!(parsed("0.0000").scaled(), 0);
    }

    #[test]
    fn rejects_bare_decimal_point_and_leading_plus() {
        assert!(matches!(
            Amount::parse("."),
            Err(ProcessingError::InvalidAmount(_))
        ));
        assert!(matches!(
            Amount::parse("+1"),
            Err(ProcessingError::InvalidAmount(_))
        ));
    }

    #[test]
    fn preserves_error_precedence_and_payload() {
        assert!(matches!(
            Amount::parse("-"),
            Err(ProcessingError::NegativeAmount(value)) if value == "-"
        ));
        assert!(matches!(
            Amount::parse("+"),
            Err(ProcessingError::InvalidAmount(value)) if value == "+"
        ));
        assert!(matches!(
            Amount::parse("1.23456"),
            Err(ProcessingError::TooManyDecimals(value)) if value == "1.23456"
        ));
        assert!(matches!(
            Amount::parse("922337203685477.5808"),
            Err(ProcessingError::Overflow)
        ));
        assert!(matches!(
            Amount::parse("  -1.0  "),
            Err(ProcessingError::NegativeAmount(value)) if value == "  -1.0  "
        ));
    }

    #[test]
    fn renders_boundary_and_interior_zero_values() {
        let values = [
            (0, "0.0"),
            (1, "0.0001"),
            (-1, "-0.0001"),
            (10, "0.001"),
            (-10, "-0.001"),
            (100, "0.01"),
            (-100, "-0.01"),
            (1000, "0.1"),
            (-1000, "-0.1"),
            (10_000, "1.0"),
            (-10_000, "-1.0"),
            (1203, "0.1203"),
            (-1203, "-0.1203"),
            (i64::MIN, "-922337203685477.5808"),
            (i64::MAX, "922337203685477.5807"),
        ];

        for (scaled, expected) in values {
            assert_eq!(
                Amount::from_scaled(scaled).to_string(),
                expected,
                "scaled value {scaled}"
            );
        }
    }
}
