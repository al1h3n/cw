//! Test drive of everything built so far: device identity + the v1 control wire format.
//!
//! Run with:  cargo run -p net --example handshake_demo
//!
//! It simulates the very first moment of a session: an Agent creates its identity and sends a
//! `Hello`; a Console decodes it, checks the protocol version, and would reply. No network yet —
//! just the pieces that the real endpoint (step 1.3+) will wire together.

use std::path::PathBuf;

use net::Identity;
use proto::{Capabilities, Control, DeviceId, Hello, PRODUCT_NAME, PROTOCOL_VERSION, Role};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== {PRODUCT_NAME} — handshake demo (protocol v{PROTOCOL_VERSION}) ===\n");

    // 1. Device identity: created on first run, then reused. Here we use a throwaway temp file.
    let key_path: PathBuf =
        std::env::temp_dir().join(format!("cowatcher-demo-{}.key", std::process::id()));
    let agent = Identity::load_or_create(&key_path)?;
    println!("[1] Device identity");
    println!("    key file        : {}", key_path.display());
    println!(
        "    public key      : {}",
        hex(agent.public_key().as_bytes())
    );
    println!(
        "    device id       : {}  (the number a teacher types)",
        agent.device_id()
    );

    // Prove persistence: reloading the same file yields the same identity.
    let reloaded = Identity::load_or_create(&key_path)?;
    println!(
        "    reload matches  : {}",
        reloaded.device_id() == agent.device_id() && reloaded.public_key() == agent.public_key()
    );

    // 2. Parsing a device id the way the Console will accept typed input.
    println!("\n[2] Device id parsing (grouping is ignored)");
    for typed in ["123 456 789", "123-456-789", "42", "nope", "1234567890"] {
        match typed.parse::<DeviceId>() {
            Ok(id) => println!("    {typed:<12} -> {id}"),
            Err(err) => println!("    {typed:<12} -> rejected: {err}"),
        }
    }

    // 3. Build the Agent's opening Hello and show its capabilities.
    let hello = Hello {
        protocol_version: PROTOCOL_VERSION,
        role: Role::Agent,
        device_id: agent.device_id(),
        capabilities: Capabilities::SCREEN_CAPTURE
            .union(Capabilities::AUDIO)
            .union(Capabilities::REMOTE_INPUT)
            .union(Capabilities::LOCK)
            .union(Capabilities::POWER),
    };
    println!("\n[3] Agent -> Console handshake");
    println!("    role            : {:?}", hello.role);
    println!("    capability bits : 0b{:08b}", hello.capabilities.bits());
    println!(
        "    can capture     : {}",
        hello.capabilities.contains(Capabilities::SCREEN_CAPTURE)
    );
    // A Console that only offers screen capture would NOT satisfy an audio requirement:
    println!(
        "    capture-only has audio? {}",
        Capabilities::SCREEN_CAPTURE.contains(Capabilities::AUDIO)
    );

    // 4. Encode it to the wire, then decode it on the "Console" side.
    let on_wire = proto::encode(&Control::Hello(hello));
    println!("\n[4] Wire encoding (postcard)");
    println!("    encoded bytes   : {} bytes", on_wire.len());
    println!("    hex             : {}", hex(&on_wire));

    match proto::decode::<Control>(&on_wire) {
        Ok(Control::Hello(received)) => {
            println!(
                "    decoded OK      : Hello from device {}",
                received.device_id
            );
            match proto::version_compatible(received.protocol_version) {
                Ok(()) => println!("    version check   : compatible -> proceed to pairing"),
                Err(err) => println!("    version check   : {err}"),
            }
        }
        other => println!("    unexpected      : {other:?}"),
    }

    // 5. Show the failure paths a peer must handle: a future version and corrupt bytes.
    println!("\n[5] Rejection paths");
    match proto::version_compatible(PROTOCOL_VERSION + 1) {
        Ok(()) => println!("    future version  : (unexpectedly accepted)"),
        Err(err) => println!("    future version  : {err}"),
    }
    let mut corrupt = on_wire.clone();
    corrupt.push(0xFF); // trailing garbage
    println!(
        "    trailing byte   : {}",
        match proto::decode::<Control>(&corrupt) {
            Ok(_) => "(unexpectedly accepted)".to_string(),
            Err(err) => format!("rejected: {err}"),
        }
    );

    // 6. Ping/Pong liveness round-trip.
    let ping = proto::encode(&Control::Ping(0xC0FFEE));
    if let Ok(Control::Ping(nonce)) = proto::decode::<Control>(&ping) {
        let pong = proto::encode(&Control::Pong(nonce));
        if let Ok(Control::Pong(echo)) = proto::decode::<Control>(&pong) {
            println!(
                "\n[6] Liveness    : Ping(0x{nonce:X}) -> Pong(0x{echo:X})  match={}",
                nonce == echo
            );
        }
    }

    // 7. Pairing: the Console shows a code; a device offers codes; the Console pins the good one.
    use net::{PairingCode, PairingSession, TrustStore};
    let now_ms = 1_000_000; // the transport supplies real time; fixed here for a repeatable demo
    let shown = PairingCode::from_u32(482_915).ok_or("482915 must be a valid 6-digit code")?;
    let mut session = PairingSession::new(shown, now_ms);
    let mut trust = TrustStore::new();
    println!("\n[7] Pairing (Console shows {shown}, code lives 5 min, 5 tries)");

    let wrong: PairingCode = "000000".parse()?;
    println!(
        "    wrong 000000    : {}",
        describe(session.verify(wrong, now_ms))
    );
    println!("    attempts left   : {}", session.attempts_left());

    match session.verify(shown, now_ms) {
        Ok(()) => {
            trust.pin(agent.public_key().as_bytes());
            println!("    correct {shown} : accepted -> pinned this device's key");
        }
        Err(err) => println!("    correct {shown} : unexpectedly {err}"),
    }
    println!(
        "    device trusted  : {}",
        trust.is_trusted(agent.public_key().as_bytes())
    );
    println!(
        "    replay {shown}  : {}",
        describe(session.verify(shown, now_ms))
    );
    let expired = now_ms + net::pairing::CODE_TTL_MS;
    let mut fresh = PairingSession::new(shown, now_ms);
    println!(
        "    after 5 min     : {}",
        describe(fresh.verify(shown, expired))
    );

    let _ = std::fs::remove_file(&key_path);
    println!("\n=== done ===");
    Ok(())
}

fn describe(result: Result<(), proto::PairRejection>) -> String {
    match result {
        Ok(()) => "accepted".to_string(),
        Err(reason) => format!("rejected: {reason}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
