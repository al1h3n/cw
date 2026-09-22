//! The broadcast engine, shared by the desktop console and the web dashboard.
//!
//! Presenting a source (a monitor or a window) to a set of PCs is the same work whether the teacher is
//! at the Tauri window or in a browser, so it lives here once and takes an [`Emitter`] for its progress
//! events. Several presentations can run at once to disjoint sets of PCs (a PC shows one at a time);
//! each has its own capture thread and parallel fan-out.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use crate::events::Emitter;
use crate::manager::DeviceManager;

/// A running broadcast: the stop flag its capture thread watches, and who is receiving it (so it can
/// be taken off exactly those screens when it ends). `label` is a short source name for the banner.
struct BroadcastHandle {
    id: u64,
    stop: Arc<AtomicBool>,
    targets: Vec<String>,
    label: String,
}

/// One running broadcast, for the "you are presenting" banners.
#[derive(serde::Serialize)]
pub struct BroadcastStatus {
    pub id: u64,
    pub targets: usize,
    pub label: String,
}

/// What a teacher chose to present, from the picker.
pub struct StartParams {
    pub source_kind: String,
    pub source_id: u64,
    pub source_title: String,
    pub width: u16,
    pub locked: bool,
    /// `"desktop"` or `"stop"`: what to do if the shared *window* closes or stays uncapturable.
    pub on_close: String,
    pub targets: Vec<String>,
}

/// Every broadcast currently running, and the id counter that keeps them distinct.
pub struct Broadcasts {
    list: Mutex<Vec<BroadcastHandle>>,
    next_id: AtomicU64,
}

impl Default for Broadcasts {
    fn default() -> Self {
        Self::new()
    }
}

impl Broadcasts {
    #[must_use]
    pub fn new() -> Self {
        Self {
            list: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Every presentation currently running (one banner each).
    #[must_use]
    pub fn status(&self) -> Vec<BroadcastStatus> {
        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|h| BroadcastStatus {
                id: h.id,
                targets: h.targets.len(),
                label: h.label.clone(),
            })
            .collect()
    }

    /// Starts broadcasting a source to the chosen PCs, optionally locking them onto it.
    ///
    /// # Errors
    /// If no PCs were chosen.
    pub async fn start(
        &self,
        manager: Arc<DeviceManager>,
        emitter: Emitter,
        params: StartParams,
    ) -> Result<(), String> {
        let StartParams {
            source_kind,
            source_id,
            source_title,
            width,
            locked,
            on_close,
            targets,
        } = params;
        if targets.is_empty() {
            return Err("choose at least one PC to broadcast to".into());
        }
        // A PC can only show one broadcast at a time, so end any *overlapping* broadcast — but leave
        // the others running, which is what lets a teacher present to disjoint groups at once.
        self.stop_overlapping(&manager, &targets).await;

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let stop = Arc::new(AtomicBool::new(false));
        // Set when a window source is gone and `on_close == "stop"`, so the fan-out can tell the UI.
        let source_lost = Arc::new(AtomicBool::new(false));
        let width = width.clamp(320, 3840);
        let label = if source_title.trim().is_empty() {
            if source_kind == "monitor" {
                format!("Display {}", source_id + 1)
            } else {
                "window".to_string()
            }
        } else {
            source_title
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);

        // Capture thread: owns the monitor capturer so it never crosses a thread boundary. It skips
        // frames identical to the last one it sent (a still teacher screen costs no network or client
        // CPU), re-sending an unchanged frame only every KEEPALIVE so a client that reconnects mid-
        // broadcast still catches up within a second or two.
        {
            let stop = Arc::clone(&stop);
            let source_lost = Arc::clone(&source_lost);
            let start_kind = source_kind.clone();
            std::thread::spawn(move || {
                /// How often an unchanged (or held) frame is re-sent, so a client that joins mid-
                /// broadcast catches up within a second or two.
                const KEEPALIVE: Duration = Duration::from_millis(1500);
                let mut kind = start_kind;
                let mut src = source_id;
                let mut monitor = if kind == "monitor" {
                    media::ThumbnailCapturer::new().ok()
                } else {
                    None
                };
                let mut last: Option<Vec<u8>> = None;
                let mut last_sent = std::time::Instant::now()
                    .checked_sub(Duration::from_secs(10))
                    .unwrap_or_else(std::time::Instant::now);
                while !stop.load(Ordering::SeqCst) {
                    let frame = if kind == "window" {
                        media::window_capture::capture_window_jpeg(src, width).ok()
                    } else {
                        let index = u8::try_from(src).unwrap_or(0);
                        // A presentation is worth a sharper JPEG than an idle grid thumbnail.
                        monitor
                            .as_mut()
                            .and_then(|c| c.capture_jpeg(index, width, 78).ok())
                    };
                    match frame {
                        Some(jpeg) => {
                            let changed = last.as_deref() != Some(jpeg.as_slice());
                            if changed || last_sent.elapsed() >= KEEPALIVE {
                                if tx.blocking_send(jpeg.clone()).is_err() {
                                    break; // receiver gone
                                }
                                last = Some(jpeg);
                                last_sent = std::time::Instant::now();
                            }
                        }
                        None if kind == "window" => {
                            if media::window_capture::window_alive(src) {
                                if let Some(held) = last.clone()
                                    && last_sent.elapsed() >= KEEPALIVE
                                {
                                    if tx.blocking_send(held).is_err() {
                                        break;
                                    }
                                    last_sent = std::time::Instant::now();
                                }
                            } else if on_close == "desktop" {
                                let mut cap = media::ThumbnailCapturer::new().ok();
                                let index = cap
                                    .as_ref()
                                    .and_then(|c| {
                                        c.monitors().iter().find(|m| m.primary).map(|m| m.index)
                                    })
                                    .unwrap_or(0);
                                monitor = cap.take();
                                kind = "monitor".to_string();
                                src = u64::from(index);
                            } else {
                                source_lost.store(true, Ordering::SeqCst);
                                break;
                            }
                        }
                        None => {}
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            });
        }

        // Fan-out task: push each captured frame to every target PC **in parallel** (a JoinSet), so
        // one frame reaches ten clients in about one device-refresh instead of piling up target-by-
        // target. It also tells the UI when a PC drops the broadcast, and when the whole broadcast
        // ended because its source window went away.
        {
            let manager = Arc::clone(&manager);
            let targets = targets.clone();
            let stop = Arc::clone(&stop);
            let source_lost = Arc::clone(&source_lost);
            let emitter = emitter.clone();
            let lost_id = id;
            tokio::spawn(async move {
                let mut showing: std::collections::HashMap<String, bool> =
                    targets.iter().map(|t| (t.clone(), false)).collect();
                while let Some(jpeg) = rx.recv().await {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let mut set = tokio::task::JoinSet::new();
                    for target in &targets {
                        let manager = Arc::clone(&manager);
                        let target = target.clone();
                        let jpeg = jpeg.clone();
                        set.spawn(async move {
                            let now = matches!(
                                manager.show_broadcast(&target, jpeg.clone(), locked).await,
                                Ok((true, _))
                            );
                            manager
                                .set_broadcast_frame(&target, if now { Some(&jpeg) } else { None });
                            (target, now)
                        });
                    }
                    while let Some(joined) = set.join_next().await {
                        let Ok((target, now)) = joined else { continue };
                        let was = showing.get(&target).copied().unwrap_or(false);
                        if was && !now {
                            emitter.emit("cowatcher://broadcast-ended", target.clone());
                        }
                        showing.insert(target, now);
                    }
                }
                if source_lost.load(Ordering::SeqCst) {
                    for target in &targets {
                        let _ = manager.stop_broadcast(target).await;
                        manager.set_broadcast_frame(target, None);
                    }
                    emitter.emit("cowatcher://broadcast-source-lost", lost_id);
                }
            });
        }

        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(BroadcastHandle {
                id,
                stop,
                targets,
                label,
            });
        Ok(())
    }

    /// Stops one broadcast by `id`, or **all** of them when `id` is absent, clearing each off its PCs.
    pub async fn stop(&self, manager: &DeviceManager, id: Option<u64>) {
        let ending = self.take(|h| id.is_none_or(|want| h.id == want));
        Self::clear(manager, ending).await;
    }

    /// Ends any broadcast that shares a PC with `targets`, leaving the rest running.
    async fn stop_overlapping(&self, manager: &DeviceManager, targets: &[String]) {
        let wanted: std::collections::HashSet<&str> = targets.iter().map(String::as_str).collect();
        let ending = self.take(|h| h.targets.iter().any(|t| wanted.contains(t.as_str())));
        Self::clear(manager, ending).await;
    }

    /// Removes the handles matching `keep_ending` from the list and returns them.
    fn take(&self, keep_ending: impl Fn(&BroadcastHandle) -> bool) -> Vec<BroadcastHandle> {
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let mut taken = Vec::new();
        list.retain(|h| {
            if keep_ending(h) {
                taken.push(BroadcastHandle {
                    id: h.id,
                    stop: Arc::clone(&h.stop),
                    targets: h.targets.clone(),
                    label: h.label.clone(),
                });
                false
            } else {
                true
            }
        });
        taken
    }

    /// Signals each ending broadcast to stop and takes it off every screen it was on.
    async fn clear(manager: &DeviceManager, ending: Vec<BroadcastHandle>) {
        for handle in ending {
            handle.stop.store(true, Ordering::SeqCst);
            for target in &handle.targets {
                let _ = manager.stop_broadcast(target).await;
                manager.set_broadcast_frame(target, None);
            }
        }
    }
}
