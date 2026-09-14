//! End-to-end pairing over a real iroh transport, two endpoints in one process.
//!
//! These are **network end-to-end** tests: they bring the console endpoint online (reaching iroh's
//! relays) and dial by full address, so they need connectivity. They are `#[ignore]`d by default so
//! offline CI stays green; run them explicitly:
//!
//! ```text
//! cargo test -p net --test pairing_over_iroh -- --ignored
//! ```
//!
//! They build their own multi-threaded Tokio runtime (iroh's background actor needs one; the default
//! `#[tokio::test]` current-thread runtime stalls it). Automated proof of Phase 1.3b; the
//! two-physical-PC and IP-change runs stay manual (docs/PLAN.md). The pairing *logic* is covered
//! offline by unit tests in `net::pairing`.
// Test-only file: panicking on setup failure is the right behaviour.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{future::Future, path::PathBuf, time::Duration};

use net::{
    Identity, PairingCode, PairingSession, TrustStore, agent_request_pairing, bind,
    console_accept_pairing, endpoint::EndpointError,
};
use proto::PairRejection;

const DIAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `fut` on a fresh multi-threaded runtime, matching how the real binaries run.
fn run<F: Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(fut)
}

/// A device identity backed by a unique temp key file, cleaned up on drop.
struct TempIdentity {
    identity: Identity,
    path: PathBuf,
}

impl TempIdentity {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "cowatcher-it-{tag}-{}-{nanos}.key",
            std::process::id()
        ));
        let identity = Identity::load_or_create(&path).expect("create identity");
        Self { identity, path }
    }
}

impl Drop for TempIdentity {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A welcome for the tests: a real Argon2id hash of a known password.
fn test_welcome() -> proto::Welcome {
    proto::Welcome {
        room: "Test Lab".to_string(),
        room_secret: net::RoomPassword::from_text("TESTPASSWORD")
            .hash()
            .expect("hash")
            .as_str()
            .to_string(),
    }
}

#[test]
#[ignore = "network end-to-end; run with --ignored"]
fn correct_code_pairs_and_both_sides_pin_each_other() {
    run(async {
        let console_id = TempIdentity::new("console");
        let agent_id = TempIdentity::new("agent");

        let console_ep = bind(&console_id.identity).await.expect("bind console");
        let agent_ep = bind(&agent_id.identity).await.expect("bind agent");
        console_ep.online().await; // make the console's address dialable
        let console_addr = console_ep.addr();

        let code = PairingCode::from_u32(314_159).unwrap();
        let mut session = PairingSession::new(code, net::endpoint::now_ms());

        let mut console_trust = TrustStore::new();
        let console_task = tokio::spawn(async move {
            let result = console_accept_pairing(
                &console_ep,
                &mut session,
                &mut console_trust,
                &test_welcome(),
            )
            .await;
            (result, console_trust)
        });

        let mut agent_trust = TrustStore::new();
        let agent_view = tokio::time::timeout(
            DIAL_TIMEOUT,
            agent_request_pairing(&agent_ep, console_addr, code, &mut agent_trust),
        )
        .await
        .expect("agent pairing timed out")
        .expect("agent should be accepted");

        let (console_result, console_trust) = console_task.await.expect("console panicked");
        let console_view = console_result.expect("console should accept");

        assert_eq!(
            agent_view.public_key,
            *console_id.identity.public_key().as_bytes()
        );
        assert_eq!(
            console_view.public_key,
            *agent_id.identity.public_key().as_bytes()
        );
        assert!(agent_trust.is_trusted(console_id.identity.public_key().as_bytes()));
        assert!(console_trust.is_trusted(agent_id.identity.public_key().as_bytes()));
    });
}

#[test]
#[ignore = "network end-to-end; run with --ignored"]
fn wrong_code_is_refused_over_the_wire() {
    run(async {
        let console_id = TempIdentity::new("console2");
        let agent_id = TempIdentity::new("agent2");

        let console_ep = bind(&console_id.identity).await.expect("bind console");
        let agent_ep = bind(&agent_id.identity).await.expect("bind agent");
        console_ep.online().await;
        let console_addr = console_ep.addr();

        let shown = PairingCode::from_u32(111_111).unwrap();
        let mut session = PairingSession::new(shown, net::endpoint::now_ms());
        let mut console_trust = TrustStore::new();
        let console_task = tokio::spawn(async move {
            let result = console_accept_pairing(
                &console_ep,
                &mut session,
                &mut console_trust,
                &test_welcome(),
            )
            .await;
            (result, console_trust)
        });

        let wrong = PairingCode::from_u32(222_222).unwrap();
        let mut agent_trust = TrustStore::new();
        let agent_result = tokio::time::timeout(
            DIAL_TIMEOUT,
            agent_request_pairing(&agent_ep, console_addr, wrong, &mut agent_trust),
        )
        .await
        .expect("agent pairing timed out");

        let (console_result, console_trust) = console_task.await.expect("console panicked");

        assert!(matches!(
            agent_result,
            Err(EndpointError::Rejected(PairRejection::WrongCode))
        ));
        assert!(matches!(
            console_result,
            Err(EndpointError::Rejected(PairRejection::WrongCode))
        ));
        assert!(agent_trust.is_empty());
        assert!(console_trust.is_empty());
    });
}
