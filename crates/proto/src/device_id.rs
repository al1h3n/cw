//! The [`DeviceId`]: a short, human-typable handle for a device.
//!
//! # Why this is *derived*, not allocated
//!
//! The real identity of a device is its **32-byte Ed25519 public key** (2^256 values), which the
//! transport verifies on every connection. Two devices cannot end up with the same key, and nobody
//! hands keys out — each device makes its own.
//!
//! The `DeviceId` is only a *handle* for that key: six characters a teacher can read aloud down a
//! corridor. Because it is **derived from the key by hashing**, not allocated from a counter:
//!
//! * there is no central table to lock and no "does this id exist yet?" round trip — the classic
//!   auto-increment problem (two devices racing for the same integer) simply cannot happen here;
//! * a device knows its own id offline, before it has ever met a Console or a Hub;
//! * the same device always shows the same id, so a teacher's written-down id keeps working.
//!
//! Six Crockford base32 characters give 32^6 = 1_073_741_824 possibilities. Collisions are therefore
//! *possible* but harmless and detectable: the id is never used to authorise anything, the public key
//! is. Within one school (say 500 devices) the chance of any clash is about 500^2 / (2 * 32^6) ≈
//! 0.01 %, and the Hub disambiguates the rare global clash by mapping id → key.
//!
//! Locally generated *records* — recording segments, audit rows — have the opposite need: many ids
//! per second, no key to derive from, and a useful sort order. Those use [`crate::RecordId`]
//! (UUIDv7), which is time-ordered and needs no coordination either.
//!
//! The alphabet is [Crockford base32](https://www.crockford.com/base32.html): digits plus letters
//! with `I`, `L`, `O` and `U` removed. That kills the `0`/`O` and `1`/`I`/`l` confusions that matter
//! when an id is read out loud, and avoids accidental rude words.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// Crockford base32 digits, in value order. No `I`, `L`, `O` or `U`.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A device's short, human-typable identifier: six Crockford base32 characters, e.g. `K7M2Q9`.
///
/// Derive it from a public key with [`DeviceId::from_public_key`]. Parsing is deliberately forgiving
/// — lower case, spaces and dashes are fine, and the look-alikes `I`/`i`/`L`/`l` read as `1` and
/// `O`/`o` as `0` — so a mistyped-but-obvious id still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceId(u32);

impl DeviceId {
    /// Number of characters in the printed form.
    pub const SYMBOLS: usize = 6;
    /// One past the largest valid value: `32^6`.
    pub const MODULUS: u32 = 1 << (5 * Self::SYMBOLS as u32); // 32^6 = 2^30

    /// Derives the ID from a 32-byte public key.
    ///
    /// Folds the whole key with FNV-1a rather than taking a slice of it, so every byte of the key
    /// affects the id. An Ed25519 public key is uniformly random, so ids spread evenly. Deterministic:
    /// the same key always yields the same id.
    #[must_use]
    pub fn from_public_key(public_key: &[u8; 32]) -> Self {
        // FNV-1a (64-bit). Tiny, dependency-free, and good enough to spread a already-random input.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in public_key {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        Self((hash % u64::from(Self::MODULUS)) as u32)
    }

    /// The numeric value, in `0..MODULUS`.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Wraps a raw value if it is in range.
    #[must_use]
    pub const fn from_u32(value: u32) -> Option<Self> {
        if value < Self::MODULUS {
            Some(Self(value))
        } else {
            None
        }
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = [b'0'; DeviceId::SYMBOLS];
        let mut value = self.0;
        // Most significant character first, so the printed form sorts the same way as the number.
        for slot in out.iter_mut().rev() {
            *slot = ALPHABET[(value % 32) as usize];
            value /= 32;
        }
        f.write_str(std::str::from_utf8(&out).unwrap_or("??????"))
    }
}

/// Error returned when a string cannot be parsed as a [`DeviceId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceIdParseError {
    /// The text contained a character that is not part of the alphabet.
    #[error("device id may contain only letters, digits, spaces and dashes (no I, L, O or U)")]
    InvalidCharacter,
    /// There were no id characters at all.
    #[error("device id is empty")]
    Empty,
    /// More than [`DeviceId::SYMBOLS`] characters were given.
    #[error("device id has more than {} characters", DeviceId::SYMBOLS)]
    TooLong,
}

/// Maps one character to its base32 value, applying Crockford's look-alike rules.
fn symbol_value(ch: char) -> Option<u32> {
    let upper = ch.to_ascii_uppercase();
    match upper {
        // Crockford: these are confusable with 1 and 0 when read aloud or handwritten.
        'I' | 'L' => Some(1),
        'O' => Some(0),
        _ => ALPHABET
            .iter()
            .position(|c| *c == upper as u8)
            .map(|p| p as u32),
    }
}

impl FromStr for DeviceId {
    type Err = DeviceIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut value: u32 = 0;
        let mut symbols = 0usize;
        for ch in s.chars() {
            if ch == ' ' || ch == '-' {
                continue; // grouping separators a human might type
            }
            let digit = symbol_value(ch).ok_or(DeviceIdParseError::InvalidCharacter)?;
            symbols += 1;
            if symbols > Self::SYMBOLS {
                return Err(DeviceIdParseError::TooLong);
            }
            // Cannot overflow: at most six base32 digits keeps this below 2^30.
            value = value * 32 + digit;
        }
        if symbols == 0 {
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
    fn from_public_key_stays_in_range() {
        for seed in [0u8, 1, 0x7F, 0xFF] {
            assert!(DeviceId::from_public_key(&[seed; 32]).as_u32() < DeviceId::MODULUS);
        }
    }

    #[test]
    fn every_byte_of_the_key_changes_the_id() {
        // A previous version hashed only the first eight bytes, so two keys differing later were
        // indistinguishable. Guard against that regression.
        let mut a = [3u8; 32];
        let mut b = [3u8; 32];
        b[31] = 4;
        assert_ne!(DeviceId::from_public_key(&a), DeviceId::from_public_key(&b));
        a[16] = 9;
        assert_ne!(DeviceId::from_public_key(&a), DeviceId::from_public_key(&b));
    }

    #[test]
    fn display_is_always_six_symbols() {
        assert_eq!(DeviceId(0).to_string(), "000000");
        assert_eq!(DeviceId(1).to_string(), "000001");
        assert_eq!(DeviceId(31).to_string(), "00000Z");
        assert_eq!(DeviceId(32).to_string(), "000010");
        assert_eq!(DeviceId(DeviceId::MODULUS - 1).to_string(), "ZZZZZZ");
    }

    #[test]
    fn display_never_uses_an_ambiguous_letter() {
        for value in [0u32, 1, 12_345, 987_654, DeviceId::MODULUS - 1] {
            let text = DeviceId(value).to_string();
            assert!(
                !text.contains(['I', 'L', 'O', 'U']),
                "{text} contains a look-alike letter"
            );
        }
    }

    #[test]
    fn display_parse_round_trip() {
        for seed in 0..32u8 {
            let id = DeviceId::from_public_key(&[seed; 32]);
            assert_eq!(id.to_string().parse::<DeviceId>().unwrap(), id);
        }
    }

    #[test]
    fn parse_accepts_lower_case_spaces_and_dashes() {
        let id: DeviceId = "K7M2Q9".parse().unwrap();
        assert_eq!("k7m2q9".parse::<DeviceId>().unwrap(), id);
        assert_eq!("K7M 2Q9".parse::<DeviceId>().unwrap(), id);
        assert_eq!("k7m-2q9".parse::<DeviceId>().unwrap(), id);
    }

    #[test]
    fn parse_forgives_the_classic_look_alikes() {
        // Someone reading "0" as "O" or "1" as "I"/"l" still reaches the right device.
        assert_eq!(
            "O12345".parse::<DeviceId>().unwrap(),
            "012345".parse::<DeviceId>().unwrap()
        );
        assert_eq!(
            "I2345Z".parse::<DeviceId>().unwrap(),
            "12345Z".parse::<DeviceId>().unwrap()
        );
        assert_eq!(
            "l2345Z".parse::<DeviceId>().unwrap(),
            "12345Z".parse::<DeviceId>().unwrap()
        );
    }

    #[test]
    fn parse_rejects_punctuation() {
        assert_eq!(
            "12*45Z".parse::<DeviceId>(),
            Err(DeviceIdParseError::InvalidCharacter)
        );
    }

    #[test]
    fn parse_rejects_too_many_symbols() {
        assert_eq!(
            "ABCDEFG".parse::<DeviceId>(),
            Err(DeviceIdParseError::TooLong)
        );
    }

    #[test]
    fn parse_rejects_empty() {
        assert_eq!("   ".parse::<DeviceId>(), Err(DeviceIdParseError::Empty));
    }

    #[test]
    fn ids_spread_across_the_space_rather_than_clustering() {
        // 1000 distinct keys should give ~1000 distinct ids; a bad derivation would collide a lot.
        let mut seen = std::collections::HashSet::new();
        for n in 0..1000u32 {
            let mut key = [0u8; 32];
            key[..4].copy_from_slice(&n.to_le_bytes());
            seen.insert(DeviceId::from_public_key(&key));
        }
        assert!(
            seen.len() > 995,
            "only {} distinct ids for 1000 keys",
            seen.len()
        );
    }
}
