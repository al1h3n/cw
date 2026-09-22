//! Tools to sync a classroom to a hosted Co-watcher dashboard — prepared, but not yet talking to a
//! real server.
//!
//! The paid tier (AGENTS.md D1) will let a teacher watch a classroom from a web dashboard hosted in
//! the cloud. That server does not exist yet, so this module builds the exact typed [`ClassroomSnapshot`]
//! that would be pushed, and offers [`SyncClient::push`]/[`SyncClient::pull`] that call a configured
//! endpoint. With no endpoint configured they return a clear "not configured" error rather than
//! pretending to sync. When the hosted endpoint is known, only the URL and the wire path settle — the
//! snapshot shape and the call sites do not change.
//!
//! **Only metadata is ever sent** — classroom and room *names*, device ids/labels/status and the
//! blocklist. Never a device key, never the room password, never a screenshot. Those stay on this
//! machine (D9/D10), so a dashboard can show the fleet without becoming a place secrets can leak.

use serde::{Deserialize, Serialize};

/// One device as the dashboard would list it (no keys, no screen bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceSummary {
    pub device_id: String,
    pub name: Option<String>,
    /// `live`, `offline`, `connecting`, …, matching [`crate::manager::DeviceStatus`] lowercased.
    pub status: String,
}

/// Everything about one classroom the dashboard needs — and nothing sensitive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassroomSnapshot {
    /// Wire version, so the server can evolve without silently misreading old clients.
    pub version: u32,
    pub slug: String,
    pub classroom_name: String,
    pub room_name: String,
    pub devices: Vec<DeviceSummary>,
    pub blocklist: Vec<String>,
    /// Milliseconds since the Unix epoch when this snapshot was taken.
    pub taken_at_ms: u64,
}

impl ClassroomSnapshot {
    /// The current snapshot version. Bump when the shape changes.
    pub const VERSION: u32 = 1;

    /// Builds a snapshot from live console state.
    #[must_use]
    pub fn build(
        slug: &str,
        classroom_name: &str,
        room_name: &str,
        devices: &[crate::manager::DeviceView],
        blocklist: Vec<String>,
    ) -> Self {
        Self {
            version: Self::VERSION,
            slug: slug.to_string(),
            classroom_name: classroom_name.to_string(),
            room_name: room_name.to_string(),
            devices: devices
                .iter()
                .map(|d| DeviceSummary {
                    device_id: d.device_id.clone(),
                    name: d.name.clone(),
                    status: format!("{:?}", d.status).to_lowercase(),
                })
                .collect(),
            blocklist,
            taken_at_ms: net::endpoint::now_ms(),
        }
    }
}

/// What the settings/sync UI shows: whether the cloud is reachable to sync to.
#[derive(Debug, Clone, Serialize)]
pub struct CloudStatus {
    /// True once an endpoint URL and a licence key are both configured.
    pub configured: bool,
    /// The endpoint that would be used (empty when not configured).
    pub endpoint: String,
    /// A human message for the card ("cloud sync is not configured yet", etc.).
    pub detail: String,
}

/// Pushes and pulls classroom snapshots to a hosted dashboard endpoint.
///
/// The endpoint base URL and licence key come from the subscription store — the same place the paid
/// dashboard link and key already live — so a teacher configures the cloud in exactly one spot.
pub struct SyncClient {
    endpoint: String,
    license: Option<String>,
}

impl SyncClient {
    /// `endpoint` is the dashboard base URL (may be empty); `license` is the sealed-then-read key.
    #[must_use]
    pub fn new(endpoint: String, license: Option<String>) -> Self {
        Self {
            endpoint: endpoint.trim().trim_end_matches('/').to_string(),
            license,
        }
    }

    /// Whether both an endpoint and a licence key are present.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        !self.endpoint.is_empty() && self.license.as_deref().is_some_and(|k| !k.is_empty())
    }

    /// A status for the UI card.
    #[must_use]
    pub fn status(&self) -> CloudStatus {
        CloudStatus {
            configured: self.is_configured(),
            endpoint: self.endpoint.clone(),
            detail: if self.is_configured() {
                "ready to sync".into()
            } else if self.endpoint.is_empty() {
                "cloud sync is not configured yet — set a dashboard URL in the subscription card"
                    .into()
            } else {
                "add a licence key to enable cloud sync".into()
            },
        }
    }

    /// The URL one classroom's snapshot lives at on the server.
    fn snapshot_url(&self, slug: &str) -> String {
        format!("{}/api/classrooms/{}/snapshot", self.endpoint, slug)
    }

    /// Uploads a snapshot. Errors clearly when the cloud is not configured.
    ///
    /// # Errors
    /// If not configured, or the request fails / is rejected.
    pub async fn push(&self, snapshot: &ClassroomSnapshot) -> Result<(), String> {
        let license = self.require()?;
        let response = reqwest::Client::new()
            .put(self.snapshot_url(&snapshot.slug))
            .bearer_auth(license)
            .json(snapshot)
            .send()
            .await
            .map_err(|e| format!("could not reach the dashboard: {e}"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "the dashboard rejected the sync: HTTP {}",
                response.status()
            ))
        }
    }

    /// Fetches the last snapshot the server holds for a classroom.
    ///
    /// # Errors
    /// If not configured, or the request fails / is rejected.
    pub async fn pull(&self, slug: &str) -> Result<ClassroomSnapshot, String> {
        let license = self.require()?;
        let response = reqwest::Client::new()
            .get(self.snapshot_url(slug))
            .bearer_auth(license)
            .send()
            .await
            .map_err(|e| format!("could not reach the dashboard: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("the dashboard returned HTTP {}", response.status()));
        }
        response
            .json()
            .await
            .map_err(|e| format!("the dashboard sent an unreadable snapshot: {e}"))
    }

    /// The licence key, or a clear error when the cloud is not configured.
    fn require(&self) -> Result<&str, String> {
        if self.endpoint.is_empty() {
            return Err("cloud sync is not configured yet (no dashboard URL)".into());
        }
        self.license
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                "cloud sync needs a licence key (add one in the subscription card)".into()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_carries_metadata_and_no_secrets() {
        let devices = vec![crate::manager::DeviceView {
            device_id: "K7M2Q9".into(),
            name: Some("Front left".into()),
            key: "deadbeef".into(),
            status: crate::manager::DeviceStatus::Live,
            screen: Some("data:image/jpeg;base64,AAA".into()),
            detail: None,
            monitors: vec![],
            monitor: 0,
            macs: vec![],
            ip: None,
            last_action: None,
        }];
        let snap = ClassroomSnapshot::build(
            "lab-7",
            "Lab 7",
            "Classroom",
            &devices,
            vec!["game.exe".into()],
        );
        assert_eq!(snap.version, ClassroomSnapshot::VERSION);
        assert_eq!(snap.devices[0].status, "live");
        assert_eq!(snap.devices[0].name.as_deref(), Some("Front left"));
        // The serialized snapshot must never carry the endpoint key or a screenshot.
        let json = serde_json::to_string(&snap).unwrap();
        assert!(!json.contains("deadbeef"), "device key must not be synced");
        assert!(!json.contains("base64"), "screens must not be synced");
    }

    #[test]
    fn not_configured_without_endpoint_or_licence() {
        assert!(!SyncClient::new(String::new(), None).is_configured());
        assert!(!SyncClient::new("https://d.example".into(), None).is_configured());
        assert!(!SyncClient::new("https://d.example".into(), Some(String::new())).is_configured());
        assert!(SyncClient::new("https://d.example".into(), Some("LIC".into())).is_configured());
    }

    #[tokio::test]
    async fn push_without_config_errors_clearly_rather_than_pretending() {
        let client = SyncClient::new(String::new(), None);
        let snap = ClassroomSnapshot::build("default", "Classroom", "Classroom", &[], vec![]);
        let err = client.push(&snap).await.expect_err("must refuse");
        assert!(err.contains("not configured"), "got: {err}");
    }
}
