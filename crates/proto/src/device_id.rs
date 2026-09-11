//! The [`DeviceId`]: a short, human-typable handle for a device.
//!
//! It is nine decimal digits derived from the device's public key. It is a **lookup handle, not a
//! secret and not the identity** — the 32-byte public key is the real identity, verified in the
//! transport handshake. Nine digits cannot be unique across every device in the world, so the Hub
//! resolves the rare collision by mapping an ID to the right public key. Anyone may know an ID.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// A device's short, human-typable identifier: exactly nine decimal digits (`0` – `999_999_999`).
///
/// Derive it from a public key with [`DeviceId::from_public_key`]. Displaying it always yields nine
/// digits (zero-padded); parsing accepts spaces and dashes so a teacher can type `123 456 789`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceId(u32);

impl DeviceId {
    /// Number of decimal digits in the printed form.
    pub const DIGITS: usize = 9;
    /// One past the largest valid value; every `DeviceId` is strictly below this.
    pub const MODULUS: u32 = 1_000_000_000;

    /// Derives the ID from a 32-byte public key.
    ///
    /// Uses the first eight key bytes as a little-endian integer, reduced modulo [`Self::MODULUS`].
    /// An Ed25519 public key is uniformly random, so this spreads IDs evenly across the nine-digit
    /// space. It is deterministic: the same key always yields the same ID.
    #[must_use]
    pub fn from_public_key(public_key: &[u8; 32]) -> Self {
        let head = u64::from_le_bytes([
            public_key[0],
            public_key[1],
            public_key[2],
            public_key[3],
            public_key[4],
            public_key[5],
            public_key[6],
            public_key[7],
        ]);
        Self((head % u64::from(Self::MODULUS)) as u32)
    }

    /// The numeric value, in `0..MODULUS`.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:0width$}", self.0, width = Self::DIGITS)
    }
}

/// Error returned when a string cannot be parsed as a [`DeviceId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceIdParseError {
    /// The text contained a character that was not a digit, space, or dash.
    #[error("device id may contain only digits, spaces and dashes")]
    InvalidCharacter,
    /// There were no digits at all.
    #[error("device id is empty")]
    Empty,
    /// More than [`DeviceId::DIGITS`] digits were given.
    #[error("device id has more than {} digits", DeviceId::DIGITS)]
    TooLong,
}

impl FromStr for DeviceId {
    type Err = DeviceIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut value: u32 = 0;
        let mut digits = 0usize;
        for ch in s.chars() {
            match ch {
                ' ' | '-' => continue, // grouping separators a human might type
                '0'..='9' => {
                    digits += 1;
                    if digits > Self::DIGITS {
                        return Err(DeviceIdParseError::TooLong);
                    }
                    // Cannot overflow: at most nine digits keeps this below 10^9.
                    value = value * 10 + (ch as u32 - '0' as u32);
                }
                _ => return Err(DeviceIdParseError::InvalidCharacter),
            }
        }
        if digits == 0 {
            return Err(DeviceIdParseError::Empty);
        }
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_public_key_is_deterministic() {
        let key = [7u8; 32];
        assert_eq!(
            DeviceId::from_public_key(&key),
            DeviceId::from_public_key(&key)
        );
    }

    #[test]
    fn from_public_key_stays_within_nine_digits() {
        let key = [0xFF; 32]; // maximal first eight bytes
        assert!(DeviceId::from_public_key(&key).as_u32() < DeviceId::MODULUS);
    }

    #[test]
    fn different_keys_usually_differ() {
        let a = DeviceId::from_public_key(&[1u8; 32]);
        let b = DeviceId::from_public_key(&[2u8; 32]);
        assert_ne!(a, b);
    }

    #[test]
    fn display_is_always_nine_digits() {
        assert_eq!(DeviceId(42).to_string(), "000000042");
        assert_eq!(DeviceId(123_456_789).to_string(), "123456789");
    }

    #[test]
    fn display_parse_round_trip() {
        let id = DeviceId::from_public_key(&[9u8; 32]);
        assert_eq!(id.to_string().parse::<DeviceId>().unwrap(), id);
    }

    #[test]
    fn parse_accepts_spaces_and_dashes() {
        assert_eq!(
            "123 456 789".parse::<DeviceId>().unwrap(),
            DeviceId(123_456_789)
        );
        assert_eq!(
            "123-456-789".parse::<DeviceId>().unwrap(),
            DeviceId(123_456_789)
        );
    }

    #[test]
    fn parse_rejects_letters() {
        assert_eq!(
            "12x".parse::<DeviceId>(),
            Err(DeviceIdParseError::InvalidCharacter)
        );
    }

    #[test]
    fn parse_rejects_too_many_digits() {
        assert_eq!(
            "1234567890".parse::<DeviceId>(),
            Err(DeviceIdParseError::TooLong)
        );
    }

    #[test]
    fn parse_rejects_empty() {
        assert_eq!("   ".parse::<DeviceId>(), Err(DeviceIdParseError::Empty));
    }
}
