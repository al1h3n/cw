//! Fuzz the control-message decoder against arbitrary bytes: it must never panic, only Ok or Err.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = proto::decode::<proto::Control>(data);
});
