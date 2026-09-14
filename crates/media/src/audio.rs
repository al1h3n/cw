//! Listening to what a student PC is playing, and playing it on the teacher's PC.
//!
//! Capture is **loopback**: it records the sound card's output — what the student hears — not a
//! microphone. On Windows this is WASAPI loopback, which cpal enables automatically when an input
//! stream is built on an output device.
//!
//! The stream is mono and decimated towards 16 kHz before it leaves the PC, because a teacher needs
//! to hear *what* is playing (a game, a video), not studio quality. That is roughly 256 kbit/s of raw
//! PCM, which is why audio is off by default and only one PC is listened to at a time.
//!
//! ponytail: raw PCM, no codec. Opus would cut this to ~32 kbit/s and is the obvious upgrade once
//! more than one PC needs listening at once; it costs a C dependency, so it waits until measured need.
//!
//! Both types own a dedicated thread: a cpal stream is not `Send`, so it cannot be parked inside a
//! struct that crosses threads. The handles here are plain data and are safe to share.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::CaptureError;

/// Sample rate we aim for after decimation. Speech and game audio stay clearly recognisable.
const TARGET_RATE: u32 = 16_000;
/// How much audio to keep if nobody collects it, in seconds. Old audio is dropped, not queued.
const BUFFER_SECONDS: usize = 3;

/// The shape of the audio being sent: always mono, at whatever rate decimation produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    /// Samples per second.
    pub sample_rate: u32,
    /// Always 1: the stream is downmixed to mono before sending.
    pub channels: u8,
}

/// Records what this PC is playing.
pub struct AudioCapture {
    format: AudioFormat,
    buffer: Arc<Mutex<VecDeque<i16>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Starts recording the default output device's loopback.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if there is no output device or the stream cannot be built — for
    /// example on a PC with no sound card, which is normal in a lab.
    pub fn start() -> Result<Self, CaptureError> {
        let (format_tx, format_rx) = std::sync::mpsc::channel();
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let worker = {
            let (buffer, stop) = (Arc::clone(&buffer), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("audio-capture".into())
                .spawn(move || run_capture(&buffer, &stop, &format_tx))
                .map_err(|e| CaptureError(format!("could not start the audio thread: {e}")))?
        };

        // The thread reports the negotiated format, or the reason it could not start.
        match format_rx.recv() {
            Ok(Ok(format)) => Ok(Self {
                format,
                buffer,
                stop,
                worker: Some(worker),
            }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(CaptureError(
                "the audio thread stopped before it started".into(),
            )),
        }
    }

    /// The format of the samples [`take`](Self::take) returns.
    #[must_use]
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// Removes and returns everything recorded since the last call, up to `max_samples`.
    #[must_use]
    pub fn take(&self, max_samples: usize) -> Vec<i16> {
        let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        let count = buffer.len().min(max_samples);
        buffer.drain(..count).collect()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Builds the loopback stream, fills the buffer until told to stop, then tears the stream down.
fn run_capture(
    buffer: &Arc<Mutex<VecDeque<i16>>>,
    stop: &Arc<AtomicBool>,
    format_tx: &std::sync::mpsc::Sender<Result<AudioFormat, CaptureError>>,
) {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        let _ = format_tx.send(Err(CaptureError(
            "this PC has no audio output device".into(),
        )));
        return;
    };
    let supported = match device.default_output_config() {
        Ok(config) => config,
        Err(err) => {
            let _ = format_tx.send(Err(CaptureError(format!("no usable audio format: {err}"))));
            return;
        }
    };

    let input_rate = supported.sample_rate();
    let channels = supported.channels().max(1);
    // Whole-number decimation keeps this to an add and a divide per sample: no resampling library.
    let factor = (input_rate as f32 / TARGET_RATE as f32).round().max(1.0) as usize;
    let format = AudioFormat {
        sample_rate: input_rate / factor as u32,
        channels: 1,
    };
    let capacity = format.sample_rate as usize * BUFFER_SECONDS;

    let sink = Arc::clone(buffer);
    let config: cpal::StreamConfig = supported.config();
    let sample_format = supported.sample_format();

    // cpal enables WASAPI loopback because this is an *output* device opened for input.
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _: &_| {
                mix_into(&sink, data, channels as usize, factor, capacity, |s| {
                    (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
                })
            },
            |err| eprintln!("audio capture error: {err}"),
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _: &_| {
                mix_into(&sink, data, channels as usize, factor, capacity, |s| s)
            },
            |err| eprintln!("audio capture error: {err}"),
            None,
        ),
        other => {
            let _ = format_tx.send(Err(CaptureError(format!(
                "unsupported sample format {other:?}"
            ))));
            return;
        }
    };

    let stream = match stream {
        Ok(stream) => stream,
        Err(err) => {
            let _ = format_tx.send(Err(CaptureError(format!("could not open loopback: {err}"))));
            return;
        }
    };
    if let Err(err) = stream.play() {
        let _ = format_tx.send(Err(CaptureError(format!(
            "could not start loopback: {err}"
        ))));
        return;
    }
    let _ = format_tx.send(Ok(format));

    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(stream);
}

/// Downmixes to mono, decimates by `factor`, and appends to the ring buffer, dropping the oldest
/// audio if nobody is collecting it.
fn mix_into<S: Copy>(
    sink: &Arc<Mutex<VecDeque<i16>>>,
    data: &[S],
    channels: usize,
    factor: usize,
    capacity: usize,
    to_i16: impl Fn(S) -> i16,
) {
    let channels = channels.max(1);
    let mut mono = Vec::with_capacity(data.len() / (channels * factor) + 1);
    // Average each frame's channels, then keep one frame out of `factor`.
    for (index, frame) in data.chunks_exact(channels).enumerate() {
        if index % factor != 0 {
            continue;
        }
        let sum: i32 = frame.iter().map(|s| i32::from(to_i16(*s))).sum();
        mono.push((sum / channels as i32) as i16);
    }

    let mut buffer = sink.lock().unwrap_or_else(|e| e.into_inner());
    buffer.extend(mono);
    let overflow = buffer.len().saturating_sub(capacity);
    if overflow > 0 {
        buffer.drain(..overflow);
    }
}

/// Plays received audio on the teacher's PC.
pub struct AudioPlayback {
    queue: Arc<Mutex<VecDeque<i16>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AudioPlayback {
    /// Opens the default output device at `sample_rate`, mono.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if there is no output device or the stream cannot be built.
    pub fn start(sample_rate: u32) -> Result<Self, CaptureError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let worker = {
            let (queue, stop) = (Arc::clone(&queue), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("audio-playback".into())
                .spawn(move || run_playback(sample_rate, &queue, &stop, &ready_tx))
                .map_err(|e| CaptureError(format!("could not start the audio thread: {e}")))?
        };

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                queue,
                stop,
                worker: Some(worker),
            }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(CaptureError(
                "the audio thread stopped before it started".into(),
            )),
        }
    }

    /// Queues samples for playback, dropping the oldest if the teacher's PC falls behind.
    pub fn push(&self, samples: &[i16], max_queued: usize) {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.extend(samples.iter().copied());
        let overflow = queue.len().saturating_sub(max_queued);
        if overflow > 0 {
            queue.drain(..overflow);
        }
    }
}

impl Drop for AudioPlayback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_playback(
    sample_rate: u32,
    queue: &Arc<Mutex<VecDeque<i16>>>,
    stop: &Arc<AtomicBool>,
    ready_tx: &std::sync::mpsc::Sender<Result<(), CaptureError>>,
) {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        let _ = ready_tx.send(Err(CaptureError("no audio output device".into())));
        return;
    };
    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    let source = Arc::clone(queue);
    let stream = device.build_output_stream(
        config,
        move |out: &mut [f32], _: &_| {
            let mut queue = source.lock().unwrap_or_else(|e| e.into_inner());
            for slot in out.iter_mut() {
                // Silence when we have nothing: a gap is better than a stutter of old audio.
                *slot = queue
                    .pop_front()
                    .map_or(0.0, |s| f32::from(s) / f32::from(i16::MAX));
            }
        },
        |err| eprintln!("audio playback error: {err}"),
        None,
    );

    let stream = match stream {
        Ok(stream) => stream,
        Err(err) => {
            let _ = ready_tx.send(Err(CaptureError(format!("could not open playback: {err}"))));
            return;
        }
    };
    if let Err(err) = stream.play() {
        let _ = ready_tx.send(Err(CaptureError(format!(
            "could not start playback: {err}"
        ))));
        return;
    }
    let _ = ready_tx.send(Ok(()));

    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(stream);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(sink: &Arc<Mutex<VecDeque<i16>>>) -> Vec<i16> {
        sink.lock().unwrap().iter().copied().collect()
    }

    #[test]
    fn downmixes_stereo_to_mono() {
        let sink = Arc::new(Mutex::new(VecDeque::new()));
        // Two stereo frames: (100, 200) averages to 150, (-100, -200) to -150.
        mix_into(&sink, &[100i16, 200, -100, -200], 2, 1, 1000, |s| s);
        assert_eq!(drain(&sink), vec![150, -150]);
    }

    #[test]
    fn decimation_keeps_one_frame_in_factor() {
        let sink = Arc::new(Mutex::new(VecDeque::new()));
        // Mono frames 0..6 with factor 3 keeps frames 0 and 3.
        mix_into(&sink, &[0i16, 1, 2, 3, 4, 5], 1, 3, 1000, |s| s);
        assert_eq!(drain(&sink), vec![0, 3]);
    }

    #[test]
    fn old_audio_is_dropped_when_nobody_collects_it() {
        let sink = Arc::new(Mutex::new(VecDeque::new()));
        for value in 0i16..10 {
            mix_into(&sink, &[value], 1, 1, 4, |s| s);
        }
        // Only the newest four samples survive: a teacher hears "now", never a growing backlog.
        assert_eq!(drain(&sink), vec![6, 7, 8, 9]);
    }

    #[test]
    fn float_samples_are_scaled_and_clamped() {
        let sink = Arc::new(Mutex::new(VecDeque::new()));
        mix_into(&sink, &[0.0f32, 1.0, -1.0, 2.0], 1, 1, 1000, |s| {
            (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
        });
        assert_eq!(drain(&sink), vec![0, i16::MAX, -i16::MAX, i16::MAX]);
    }
}
