//! Generating a [`proto::RecordId`] (UUIDv7) for things this PC creates.
//!
//! The layout and the reasoning live in `proto::record_id`, which stays I/O-free. Reading the clock
//! and drawing random bytes are side effects, so they happen here, in a binary.

use std::time::{SystemTime, UNIX_EPOCH};

use proto::RecordId;

/// A fresh, time-ordered id for a recording or a log entry.
///
/// Two ids made in the same millisecond differ in their random half; ids made in different
/// milliseconds also sort correctly by time, which is the whole point of v7 over v4.
#[must_use]
pub fn now() -> RecordId {
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    let mut random = [0u8; 10];
    for byte in &mut random {
        *byte = rand::random();
    }
    RecordId::from_parts(unix_ms, random)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_id_is_version_7_and_carries_roughly_now() {
        let id = now();
        assert_eq!(id.version(), 7);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(
            now_ms.saturating_sub(id.unix_ms()) < 5_000,
            "the embedded time should be about now"
        );
    }

    #[test]
    fn two_ids_made_back_to_back_are_different() {
        // Same millisecond is likely here, so this exercises the random half.
        let ids: Vec<RecordId> = (0..50).map(|_| now()).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "ids must not repeat");
    }

    #[test]
    fn ids_made_later_sort_after_ids_made_earlier() {
        let first = now();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let second = now();
        assert!(first < second, "v7 ids sort by creation time");
        assert!(
            first.to_string() < second.to_string(),
            "and so does the text"
        );
    }

    #[test]
    fn the_compact_form_is_safe_to_put_in_a_file_name() {
        let name = format!("recording-{}.avi", now().to_compact());
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
            "{name} must be a safe file name"
        );
    }
}
