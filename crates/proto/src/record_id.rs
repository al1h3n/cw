//! [`RecordId`]: a UUIDv7 for things this device *creates* — recording segments, audit rows.
//!
//! # Why v7 and not an integer or v4
//!
//! * **A 4-byte auto-increment integer** is small and fast, but two writers must agree who gets the
//!   next number. That means a lock or a round trip to a central table, and an Agent that is offline
//!   (which ours often is, by design — D9) cannot take one at all.
//! * **UUIDv4** is 16 random bytes: no coordination needed and a collision is not worth worrying
//!   about (2^122 of randomness), but the values are scattered, so they sort meaninglessly and make
//!   a poor database index — every insert lands in a random page.
//! * **UUIDv7** keeps v4's "no coordination" property and puts a 48-bit millisecond Unix timestamp
//!   in the leading bytes. So ids generated later sort after ids generated earlier, as text or as
//!   bytes. Recording segments and audit rows list themselves in the right order with no extra
//!   column, and index inserts stay at the end.
//! * **UUIDv8** is the "put your own layout here" version. We do not use it: it only pays off when
//!   you must pack a tenant or shard key into the id, and it gives up interoperability to do it.
//!
//! So: derived short codes for *devices* (see [`crate::DeviceId`] — no allocation, no lock), UUIDv7
//! for *records*. Neither ever needs "is this id taken?".
//!
//! This is a small, dependency-free implementation of RFC 9562 §5.7.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// A time-ordered unique id for a locally created record (UUIDv7).
///
/// Printed in the usual hyphenated hex form, e.g. `01935f4c-1a2b-7c3d-8e4f-5a6b7c8d9e0f`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecordId([u8; 16]);

impl RecordId {
    /// Builds a v7 id from a millisecond timestamp and 10 bytes of randomness.
    ///
    /// Kept separate from [`Self::now`] so the layout can be tested with fixed inputs.
    ///
    /// Layout: 48 bits big-endian time, 4 bits version (7), 12 bits random, 2 bits variant (0b10),
    /// 62 bits random.
    #[must_use]
    pub const fn from_parts(unix_ms: u64, random: [u8; 10]) -> Self {
        let t = unix_ms.to_be_bytes(); // take the low 48 bits: t[2..8]
        let mut bytes = [0u8; 16];
        bytes[0] = t[2];
        bytes[1] = t[3];
        bytes[2] = t[4];
        bytes[3] = t[5];
        bytes[4] = t[6];
        bytes[5] = t[7];
        // Version 7 in the high nibble of byte 6, 12 random bits below it.
        bytes[6] = 0x70 | (random[0] & 0x0F);
        bytes[7] = random[1];
        // Variant 0b10 in the top two bits of byte 8.
        bytes[8] = 0x80 | (random[2] & 0x3F);
        bytes[9] = random[3];
        bytes[10] = random[4];
        bytes[11] = random[5];
        bytes[12] = random[6];
        bytes[13] = random[7];
        bytes[14] = random[8];
        bytes[15] = random[9];
        Self(bytes)
    }

    /// The 16 raw bytes.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }

    /// The embedded creation time, in milliseconds since the Unix epoch.
    #[must_use]
    pub const fn unix_ms(self) -> u64 {
        let b = self.0;
        ((b[0] as u64) << 40)
            | ((b[1] as u64) << 32)
            | ((b[2] as u64) << 24)
            | ((b[3] as u64) << 16)
            | ((b[4] as u64) << 8)
            | (b[5] as u64)
    }

    /// The UUID version nibble; always 7 for ids we generate.
    #[must_use]
    pub const fn version(self) -> u8 {
        self.0[6] >> 4
    }

    /// A short, file-name-safe form: the 32 hex characters with no hyphens.
    #[must_use]
    pub fn to_compact(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let h = |r: std::ops::Range<usize>| -> String {
            self.0[r].iter().map(|b| format!("{b:02x}")).collect()
        };
        write!(
            f,
            "{}-{}-{}-{}-{}",
            h(0..4),
            h(4..6),
            h(6..8),
            h(8..10),
            h(10..16)
        )
    }
}

/// Error parsing a [`RecordId`] from text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a valid record id")]
pub struct RecordIdParseError;

impl FromStr for RecordId {
    type Err = RecordIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex: Vec<u8> = s
            .bytes()
            .filter(|b| *b != b'-')
            .map(|b| match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                b'A'..=b'F' => Ok(b - b'A' + 10),
                _ => Err(RecordIdParseError),
            })
            .collect::<Result<_, _>>()?;
        if hex.len() != 32 {
            return Err(RecordIdParseError);
        }
        let mut bytes = [0u8; 16];
        for (slot, pair) in bytes.iter_mut().zip(hex.chunks_exact(2)) {
            *slot = (pair[0] << 4) | pair[1];
        }
        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_rfc_9562_version_and_variant() {
        let id = RecordId::from_parts(0x0192_3F4C_1A2B, [0xFF; 10]);
        assert_eq!(id.version(), 7, "version nibble must be 7");
        assert_eq!(id.as_bytes()[8] & 0xC0, 0x80, "variant bits must be 0b10");
    }

    #[test]
    fn the_timestamp_survives_a_round_trip() {
        let ms = 1_789_394_305_438u64;
        assert_eq!(RecordId::from_parts(ms, [0; 10]).unix_ms(), ms);
    }

    #[test]
    fn later_ids_sort_after_earlier_ones_even_with_smaller_random_parts() {
        // This is the whole point of v7 over v4: byte order follows time order.
        let earlier = RecordId::from_parts(1_000, [0xFF; 10]);
        let later = RecordId::from_parts(2_000, [0x00; 10]);
        assert!(earlier < later);
        assert!(earlier.to_string() < later.to_string(), "text sorts too");
    }

    #[test]
    fn display_is_the_hyphenated_uuid_form() {
        let id = RecordId::from_parts(0, [0; 10]);
        let text = id.to_string();
        assert_eq!(text.len(), 36);
        assert_eq!(text.as_bytes()[8], b'-');
        assert_eq!(text.as_bytes()[13], b'-');
        assert_eq!(text.as_bytes()[18], b'-');
        assert_eq!(text.as_bytes()[23], b'-');
        assert_eq!(
            &text[14..15],
            "7",
            "version digit is visible in the text form"
        );
    }

    #[test]
    fn display_parse_round_trip() {
        let id = RecordId::from_parts(1_789_394_305_438, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(id.to_string().parse::<RecordId>().unwrap(), id);
        assert_eq!(id.to_compact().parse::<RecordId>().unwrap(), id);
    }

    #[test]
    fn compact_form_is_safe_in_a_file_name() {
        let text = RecordId::from_parts(5, [7; 10]).to_compact();
        assert_eq!(text.len(), 32);
        assert!(text.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn parse_rejects_wrong_length_and_junk() {
        assert_eq!("abcd".parse::<RecordId>(), Err(RecordIdParseError));
        assert_eq!(
            "zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz".parse::<RecordId>(),
            Err(RecordIdParseError)
        );
    }

    #[test]
    fn distinct_random_parts_give_distinct_ids_in_the_same_millisecond() {
        let a = RecordId::from_parts(42, [1; 10]);
        let b = RecordId::from_parts(42, [2; 10]);
        assert_ne!(a, b);
    }
}
