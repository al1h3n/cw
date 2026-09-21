//! End-to-end control session over a real iroh transport (Phase 1.5).
//!
//! Network e2e, `#[ignore]`d like the pairing tests; run with:
//! `cargo test -p net --test control_over_iroh -- --ignored`.
//! Own multi-threaded runtime (iroh's actor stalls under the default current-thread test runtime).
// Test-only file: panicking on setup failure is the right behaviour.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use net::{
    AgentDevice, CaptureError, ControlSession, Identity, LocalHello, TrustStore, bind,
    endpoint::EndpointError,
};
use proto::{Capabilities, DeviceId, Monitor, ProtocolError, Role};

fn run<F: Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(fut)
}

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
            "cowatcher-ctl-{tag}-{}-{nanos}.key",
            std::process::id()
        ));
        let identity = Identity::load_or_create(&path).expect("identity");
        Self { identity, path }
    }
    fn key(&self) -> [u8; 32] {
        *self.identity.public_key().as_bytes()
    }
    fn hello(&self, role: Role, caps: Capabilities) -> LocalHello {
        LocalHello {
            role,
            device_id: DeviceId::from_public_key(&self.key()),
            capabilities: caps,
        }
    }
}

impl Drop for TempIdentity {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A capture source that counts calls and returns a tiny valid-looking JPEG (SOI…EOI markers).
#[derive(Clone, Default)]
struct FakeCapture {
    calls: Arc<AtomicU32>,
    /// The last (monitor, max_width) asked for, packed so a test can check both.
    last_request: Arc<AtomicU32>,
}

impl AgentDevice for FakeCapture {
    fn perform(&self, from: &net::PeerInfo, action: proto::Action) -> proto::ActionOutcome {
        // Refuse anything but locking, and only from a Console, so both paths cross the wire.
        match action {
            proto::Action::LockScreen if from.role == Role::Console => {
                proto::ActionOutcome::Started { delay_seconds: 0 }
            }
            _ => proto::ActionOutcome::Failed(proto::ActionFailure::NotPermitted),
        }
    }

    fn capture_thumbnail(
        &self,
        monitor: u8,
        max_width: u16,
        _quality: u8,
    ) -> Result<Vec<u8>, CaptureError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.last_request.store(
            u32::from(monitor) << 16 | u32::from(max_width),
            Ordering::SeqCst,
        );
        Ok(vec![0xFF, 0xD8, 0xFF, 0xD9])
    }

    fn monitors(&self) -> Vec<Monitor> {
        vec![
            Monitor {
                index: 0,
                width: 1920,
                height: 1080,
                primary: true,
            },
            Monitor {
                index: 1,
                width: 2560,
                height: 1440,
                primary: false,
            },
        ]
    }
}

#[test]
#[ignore = "network end-to-end; run with --ignored"]
fn trusted_console_gets_thumbnails_only_on_request() {
    run(async {
        let console_id = TempIdentity::new("console");
        let agent_id = TempIdentity::new("agent");
        let console_ep = bind(&console_id.identity).await.expect("bind console");
        let agent_ep = bind(&agent_id.identity).await.expect("bind agent");
        agent_ep.online().await;
        let agent_addr = agent_ep.addr();

        // Simulate a prior pairing: each side already trusts the other's key.
        let mut console_trust = TrustStore::new();
        console_trust.pin(&agent_id.key());
        let mut agent_trust = TrustStore::new();
        agent_trust.pin(&console_id.key());

        let capture = FakeCapture::default();
        let calls = capture.calls.clone();
        let last_request = capture.last_request.clone();
        let agent_hello = agent_id.hello(Role::Agent, Capabilities::SCREEN_CAPTURE);
        let agent_task = tokio::spawn(async move {
            let session = ControlSession::accept(&agent_ep, &agent_trust, agent_hello).await?;
            session.serve(&capture).await
        });

        let console_hello = console_id.hello(Role::Console, Capabilities::EMPTY);
        let mut session =
            ControlSession::connect(&console_ep, agent_addr, &console_trust, console_hello)
                .await
                .expect("console should connect");
        assert_eq!(session.peer().role, Role::Agent);
        assert!(
            session
                .peer()
                .capabilities
                .contains(Capabilities::SCREEN_CAPTURE)
        );
        assert_eq!(session.peer().public_key, agent_id.key());

        // No capture has happened yet — only requests trigger it.
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        for _ in 0..3 {
            let jpeg = session
                .request_thumbnail(0, 320, 60)
                .await
                .expect("thumbnail");
            assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "starts with a JPEG SOI marker");
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "one capture per request, no background capture"
        );

        // The agent reports its monitors, and the console can ask for a specific one at a chosen size.
        let monitors = session.request_monitors().await.expect("monitors");
        assert_eq!(monitors.len(), 2);
        assert!(monitors[0].primary && !monitors[1].primary);
        assert_eq!(monitors[1].width, 2560);

        session
            .request_thumbnail(1, 1280, 60)
            .await
            .expect("second monitor");
        let packed = last_request.load(Ordering::SeqCst);
        assert_eq!(
            packed >> 16,
            1,
            "the requested monitor reached the capture source"
        );
        assert_eq!(
            packed & 0xFFFF,
            1280,
            "the requested width reached the capture source"
        );

        // Actions travel as typed values and the device sees who asked.
        assert_eq!(
            session
                .perform(proto::Action::LockScreen)
                .await
                .expect("lock"),
            proto::ActionOutcome::Started { delay_seconds: 0 }
        );
        assert_eq!(
            session
                .perform(proto::Action::Shutdown { delay_seconds: 60 })
                .await
                .expect("shutdown reply"),
            proto::ActionOutcome::Failed(proto::ActionFailure::NotPermitted)
        );

        session.close();
        agent_task
            .await
            .expect("agent task")
            .expect("agent serve ends cleanly");
    });
}

#[test]
#[ignore = "network end-to-end; run with --ignored"]
fn untrusted_console_is_refused() {
    run(async {
        let console_id = TempIdentity::new("console2");
        let agent_id = TempIdentity::new("agent2");
        let console_ep = bind(&console_id.identity).await.expect("bind console");
        let agent_ep = bind(&agent_id.identity).await.expect("bind agent");
        agent_ep.online().await;
        let agent_addr = agent_ep.addr();

        // The Agent does NOT trust this Console (no prior pairing). The Console optimistically trusts
        // the Agent, so its own handshake check passes — but the Agent refuses.
        let mut console_trust = TrustStore::new();
        console_trust.pin(&agent_id.key());
        let agent_trust = TrustStore::new(); // empty: trusts nobody

        let agent_hello = agent_id.hello(Role::Agent, Capabilities::SCREEN_CAPTURE);
        let agent_task = tokio::spawn(async move {
            let capture = FakeCapture::default();
            match ControlSession::accept(&agent_ep, &agent_trust, agent_hello).await {
                Ok(session) => session.serve(&capture).await,
                Err(err) => Err(err),
            }
        });

        let console_hello = console_id.hello(Role::Console, Capabilities::EMPTY);
        // The Console may connect, but requesting a thumbnail must fail (the Agent refused and closed).
        let outcome: Result<(), EndpointError> = async {
            let mut session =
                ControlSession::connect(&console_ep, agent_addr, &console_trust, console_hello)
                    .await?;
            session.request_thumbnail(0, 320, 60).await?;
            Ok(())
        }
        .await;
        assert!(
            outcome.is_err(),
            "an untrusted console must not receive thumbnails"
        );

        let agent_result = agent_task.await.expect("agent task");
        assert!(
            matches!(
                agent_result,
                Err(EndpointError::ControlRefused(ProtocolError::Unauthorized))
            ),
            "agent must refuse the untrusted console, got {agent_result:?}",
        );
    });
}
