//! Encoding this PC's screen as a live H.264 stream for one watching teacher.
//!
//! Same shape as [`crate::recording`], and for the same reason: Direct3D and the capturer are not
//! `Send`, so the whole capture→resize→encode pipeline lives on one dedicated OS thread and hands
//! finished packets to the async side through a channel. The network never touches the encoder and
//! the encoder never waits on the network.
//!
//! Back-pressure is deliberate and one-sided: the channel is **bounded and drops the newest frame
//! when full**. If the link cannot carry what we encode, the right answer for live video is to skip
//! a frame, never to queue it — a teacher wants to see *now*, and a backlog would grow without
//! bound and arrive as slow motion.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};

use media::{
    h264::{Encoder, StreamSettings},
    resize,
};

/// How many encoded frames may wait for the network before we start dropping.
///
/// Three is about a tenth of a second at 30 fps: enough to ride out a scheduling hiccup, too little
/// to build a visible delay.
const QUEUE_FRAMES: usize = 3;

/// A running stream: owns its encoder thread.
pub struct Stream {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// Frames encoded so far, for the `serve` banner and support questions.
    frames: Arc<AtomicU64>,
    /// Frames dropped because the network could not keep up.
    dropped: Arc<AtomicU64>,
}

impl Stream {
    /// Starts capturing and encoding `monitor`, returning the settings actually used and the
    /// channel of encoded packets.
    ///
    /// # Errors
    /// A message if the screen cannot be captured or the encoder refuses the settings.
    pub fn start(
        monitor: u8,
        settings: proto::VideoSettings,
    ) -> Result<
        (
            Self,
            proto::VideoSettings,
            tokio::sync::mpsc::Receiver<Vec<u8>>,
        ),
        String,
    > {
        // Take one frame up front so the stream matches the screen's real shape, and so a PC that
        // cannot capture fails here — visibly — instead of going quiet.
        let mut capturer = media::ThumbnailCapturer::new().map_err(|e| e.to_string())?;
        let (_, src_w, src_h) = capturer.capture_bgra(monitor).map_err(|e| e.to_string())?;

        let (width, height) = resize::fit_within(src_w, src_h, settings.width, settings.height);
        let wanted = StreamSettings {
            width,
            height,
            fps: settings.fps,
            kbps: settings.kbps,
        };
        let encoder = Encoder::new(wanted).map_err(|e| e.to_string())?;
        let actual = encoder.settings();

        let (sender, receiver) = tokio::sync::mpsc::channel(QUEUE_FRAMES);
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));

        let worker = {
            let stop = Arc::clone(&stop);
            let frames = Arc::clone(&frames);
            let dropped = Arc::clone(&dropped);
            std::thread::spawn(move || {
                run(
                    capturer, encoder, monitor, actual, &sender, &stop, &frames, &dropped,
                );
            })
        };

        Ok((
            Self {
                stop,
                worker: Some(worker),
                frames,
                dropped,
            },
            proto::VideoSettings {
                width: actual.width,
                height: actual.height,
                fps: actual.fps,
                kbps: actual.kbps,
            },
            receiver,
        ))
    }

    /// How many frames have been encoded, and how many were dropped for the network.
    #[must_use]
    pub fn counts(&self) -> (u64, u64) {
        (
            self.frames.load(Ordering::Relaxed),
            self.dropped.load(Ordering::Relaxed),
        )
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The encode loop: grab, scale to the stream size, encode, hand over, wait for the next slot.
#[expect(
    clippy::too_many_arguments,
    reason = "a private worker taking exactly the state it needs, rather than a struct used once"
)]
fn run(
    mut capturer: media::ThumbnailCapturer,
    mut encoder: Encoder,
    monitor: u8,
    settings: StreamSettings,
    sender: &tokio::sync::mpsc::Sender<Vec<u8>>,
    stop: &Arc<AtomicBool>,
    frames: &Arc<AtomicU64>,
    dropped: &Arc<AtomicU64>,
) {
    let interval = settings.frame_interval();
    let mut next_slot = Instant::now();
    // The GPU scales to at most this width; anything left over is finished on the CPU, which is a
    // no-op when the mip chain already landed on the exact size.
    let max_width = u16::try_from(settings.width).unwrap_or(u16::MAX);
    // The last frame we encoded, reused when the desktop has not changed. Re-encoding identical
    // pixels costs almost nothing in H.264 and keeps the stream flowing for the decoder.
    let mut previous: Option<Vec<u8>> = None;
    // A sustained capture failure (a lock screen held for minutes) fires every frame; aggregate the
    // misses and report at most once a second so the console window is not buried in identical lines.
    let mut misses: u64 = 0;
    let mut last_miss_log = Instant::now();

    while !stop.load(Ordering::SeqCst) {
        let frame = match capturer.capture_scaled_bgra(monitor, max_width) {
            Ok(Some((pixels, w, h))) => {
                let scaled = if w == settings.width && h == settings.height {
                    pixels // the GPU already produced exactly what the encoder wants
                } else {
                    resize::area_average(&pixels, w, h, settings.width, settings.height)
                };
                if scaled.is_empty() {
                    None
                } else {
                    previous = Some(scaled.clone());
                    Some(scaled)
                }
            }
            // Nothing changed on screen: send the previous picture again so the stream stays live.
            Ok(None) => previous.clone(),
            Err(err) => {
                // One missed grab (a UAC prompt, a mode change) must not end the lesson's stream.
                misses += 1;
                if last_miss_log.elapsed() >= std::time::Duration::from_secs(1) {
                    eprintln!("stream frame missed ({misses}x, last: {err})");
                    misses = 0;
                    last_miss_log = Instant::now();
                }
                previous.clone()
            }
        };

        if let Some(scaled) = frame {
            match encoder.encode(&scaled) {
                Ok(packet) if !packet.is_empty() => {
                    frames.fetch_add(1, Ordering::Relaxed);
                    // try_send, never send: a full queue means the link is behind, and the
                    // newest frame is worth less than keeping latency low.
                    if sender.try_send(packet).is_err() {
                        dropped.fetch_add(1, Ordering::Relaxed);
                        if sender.is_closed() {
                            return; // the console went away
                        }
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    eprintln!("stream encode failed: {err}");
                    return;
                }
            }
        }

        next_slot += interval;
        let now = Instant::now();
        if next_slot > now {
            // Sleep in slices so a stop is noticed promptly.
            let mut remaining = next_slot - now;
            while remaining > std::time::Duration::ZERO && !stop.load(Ordering::SeqCst) {
                let slice = remaining.min(std::time::Duration::from_millis(20));
                std::thread::sleep(slice);
                remaining = remaining.saturating_sub(slice);
            }
        } else {
            // Encoding is slower than the requested rate; do not try to catch up in a burst.
            next_slot = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_queue_drops_rather_than_waits() {
        // The property that keeps live video live: when the link is behind, the newest frame is
        // discarded instead of queued. A bounded channel is what enforces it, and the bound must
        // stay small — three frames is about 100 ms at 30 fps.
        const _: () = assert!(QUEUE_FRAMES <= 4);

        let (sender, _receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(QUEUE_FRAMES);
        for _ in 0..QUEUE_FRAMES {
            sender
                .try_send(vec![0u8; 8])
                .expect("queue accepts up to its bound");
        }
        assert!(
            sender.try_send(vec![0u8; 8]).is_err(),
            "a full queue must refuse, so the encoder drops the frame instead of blocking"
        );
    }
}
