//! H.264 encoding and decoding for the full-resolution remote view.
//!
//! The grid uses JPEG thumbnails, which are simple and change-only. Watching *one* screen properly
//! is a different problem: at 1080p a JPEG frame is ~150 KB, so 30 fps would be 36 Mbit/s. H.264
//! sends only what changed between frames and gets the same picture in 1–3 Mbit/s — roughly 20×
//! less — which is what makes smooth full-resolution viewing possible at all on school wiring.
//!
//! This is the **OpenH264 path** from Phase-0 spike 0.5, which measured 720p30 encode at 5.5 ms and
//! decode at 1 ms on the dev PC. AGENTS.md D11 names the OS hardware encoder (Media Foundation) as
//! the eventual first choice, because it would move the work to the GPU and meet the <10 % CPU
//! budget at 1080p (§6); this software path is the documented fallback and the one that works
//! everywhere, including in a VM with no GPU encoder.
//!
//! ## Two build notes worth knowing
//!
//! * **NASM is required at build time** for OpenH264's assembly. Without it the crate still builds
//!   but silently uses a ~4× slower C path (spike 0.5 measured 22 ms instead of 5.5 ms at 720p).
//!   `docs/PLAN.md` records the portable copy under `spikes/target/tools/`.
//! * **H.264 is patent-encumbered.** OpenH264's *source* is BSD, but Cisco's royalty coverage
//!   attaches to the binaries Cisco itself publishes, not to a copy we compile. For shipping, the
//!   clean answers are the Media Foundation encoder (H.264 is already licensed as part of Windows)
//!   or Cisco's downloaded binary, exactly as Firefox does. This is recorded in `docs/FEATURES.md`
//!   rather than left as a surprise.

use openh264::{
    OpenH264API,
    decoder::Decoder as OpenDecoder,
    encoder::{
        BitRate, Encoder as OpenEncoder, EncoderConfig, FrameRate, RateControlMode, UsageType,
    },
    formats::{BgraSliceU8, YUVBuffer, YUVSource},
};

use crate::CaptureError;

/// How a stream should look. Clamped on arrival, because a remote peer chooses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSettings {
    /// Frame width in pixels. Must be even (H.264 chroma is subsampled).
    pub width: u32,
    /// Frame height in pixels. Must be even.
    pub height: u32,
    /// Target frames per second.
    pub fps: u32,
    /// Target bitrate in kbit/s.
    pub kbps: u32,
}

impl StreamSettings {
    /// Smallest sensible frame.
    pub const MIN_SIDE: u32 = 160;
    /// Largest we will encode.
    pub const MAX_WIDTH: u32 = 3840;
    /// Tallest we will encode.
    pub const MAX_HEIGHT: u32 = 2160;
    /// Slowest useful rate.
    pub const MIN_FPS: u32 = 1;
    /// Fastest we will encode; past this the encoder, not the network, is the limit.
    pub const MAX_FPS: u32 = 60;
    /// Floor for bitrate; below this 1080p is unwatchable mush.
    pub const MIN_KBPS: u32 = 200;
    /// Ceiling, so one PC cannot saturate a school uplink.
    pub const MAX_KBPS: u32 = 20_000;

    /// Clamps everything into a range the encoder can deliver, and forces even dimensions.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            // `& !1` rounds down to even: H.264's 4:2:0 chroma needs even width and height.
            width: self.width.clamp(Self::MIN_SIDE, Self::MAX_WIDTH) & !1,
            height: self.height.clamp(Self::MIN_SIDE, Self::MAX_HEIGHT) & !1,
            fps: self.fps.clamp(Self::MIN_FPS, Self::MAX_FPS),
            kbps: self.kbps.clamp(Self::MIN_KBPS, Self::MAX_KBPS),
        }
    }

    /// How long one frame lasts at this rate.
    #[must_use]
    pub fn frame_interval(self) -> std::time::Duration {
        std::time::Duration::from_micros(1_000_000 / u64::from(self.fps.max(1)))
    }
}

impl Default for StreamSettings {
    fn default() -> Self {
        // 720p30 at 2 Mbit/s: smooth, readable, and comfortable on a school LAN.
        Self {
            width: 1280,
            height: 720,
            fps: 30,
            kbps: 2_000,
        }
    }
}

/// Encodes BGRA frames into H.264 packets.
pub struct Encoder {
    encoder: OpenEncoder,
    yuv: YUVBuffer,
    settings: StreamSettings,
}

impl Encoder {
    /// Builds an encoder for `settings` (clamped first).
    ///
    /// Uses `ScreenContentRealTime`, which tunes OpenH264 for desktop content — large flat areas and
    /// sharp text rather than camera noise — and bitrate rate-control so the stream stays inside the
    /// budget a school's network can carry.
    ///
    /// # Errors
    /// [`CaptureError`] if OpenH264 refuses the configuration.
    pub fn new(settings: StreamSettings) -> Result<Self, CaptureError> {
        let settings = settings.clamped();
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(settings.kbps * 1000))
            .max_frame_rate(FrameRate::from_hz(settings.fps as f32));
        let encoder = OpenEncoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|e| CaptureError(format!("H.264 encoder: {e}")))?;
        Ok(Self {
            encoder,
            yuv: YUVBuffer::new(settings.width as usize, settings.height as usize),
            settings,
        })
    }

    /// The settings actually in use, after clamping.
    #[must_use]
    pub fn settings(&self) -> StreamSettings {
        self.settings
    }

    /// Encodes one BGRA frame, returning an Annex-B H.264 packet.
    ///
    /// `pixels` must be exactly `width * height * 4` bytes for the configured size.
    ///
    /// # Errors
    /// [`CaptureError`] if the frame is the wrong size or the encoder fails.
    pub fn encode(&mut self, pixels: &[u8]) -> Result<Vec<u8>, CaptureError> {
        let (w, h) = (self.settings.width as usize, self.settings.height as usize);
        let expected = w * h * 4;
        if pixels.len() != expected {
            return Err(CaptureError(format!(
                "frame is {} bytes, expected {expected} for {w}x{h}",
                pixels.len()
            )));
        }
        self.yuv.read_bgra8(BgraSliceU8::new(pixels, (w, h)));
        let packet = self
            .encoder
            .encode(&self.yuv)
            .map_err(|e| CaptureError(format!("H.264 encode: {e}")))?;
        Ok(packet.to_vec())
    }
}

/// Decodes H.264 packets back into BGRA frames.
pub struct Decoder {
    decoder: OpenDecoder,
    rgba: Vec<u8>,
}

impl Decoder {
    /// Creates a decoder.
    ///
    /// # Errors
    /// [`CaptureError`] if OpenH264 cannot start.
    pub fn new() -> Result<Self, CaptureError> {
        Ok(Self {
            decoder: OpenDecoder::new().map_err(|e| CaptureError(format!("H.264 decoder: {e}")))?,
            rgba: Vec::new(),
        })
    }

    /// Decodes one packet into BGRA, returning `(pixels, width, height)`.
    ///
    /// Returns `Ok(None)` when the packet carried no complete frame — normal for the first few
    /// packets of a stream, which are parameter sets rather than picture data.
    ///
    /// # Errors
    /// [`CaptureError`] if the packet is malformed.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Option<(Vec<u8>, u32, u32)>, CaptureError> {
        let Some(yuv) = self
            .decoder
            .decode(packet)
            .map_err(|e| CaptureError(format!("H.264 decode: {e}")))?
        else {
            return Ok(None);
        };
        let (width, height) = yuv.dimensions();
        self.rgba.resize(yuv.rgba8_len(), 0);
        yuv.write_rgba8(&mut self.rgba);

        // RGBA out of OpenH264, BGRA everywhere else in this crate (it is what Windows wants).
        let mut bgra = vec![0u8; width * height * 4];
        for (out, inp) in bgra.chunks_exact_mut(4).zip(self.rgba.chunks_exact(4)) {
            out[0] = inp[2];
            out[1] = inp[1];
            out[2] = inp[0];
            out[3] = 255;
        }
        Ok(Some((bgra, width as u32, height as u32)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BGRA test image with a hard vertical edge, which survives H.264 recognisably.
    fn test_frame(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..height {
            for x in 0..width {
                // Left half near-black, right half near-white.
                let v = if x < width / 2 { 16 } else { 235 };
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        pixels
    }

    #[test]
    fn settings_clamp_absurd_requests_and_force_even_sides() {
        let got = StreamSettings {
            width: 99_999,
            height: 1081, // odd
            fps: 999,
            kbps: 10,
        }
        .clamped();
        assert_eq!(got.width, StreamSettings::MAX_WIDTH);
        assert_eq!(got.height % 2, 0, "H.264 needs even dimensions");
        assert_eq!(got.fps, StreamSettings::MAX_FPS);
        assert_eq!(got.kbps, StreamSettings::MIN_KBPS);
    }

    #[test]
    fn the_frame_interval_follows_the_rate() {
        let s = StreamSettings {
            fps: 30,
            ..StreamSettings::default()
        };
        assert_eq!(s.frame_interval().as_micros(), 33_333);
    }

    #[test]
    fn a_wrongly_sized_frame_is_refused_rather_than_encoded_as_garbage() {
        let mut encoder = Encoder::new(StreamSettings {
            width: 320,
            height: 240,
            fps: 15,
            kbps: 500,
        })
        .expect("encoder");
        assert!(encoder.encode(&[0u8; 10]).is_err());
    }

    #[test]
    fn a_frame_survives_an_encode_and_decode_round_trip() {
        // The real proof the codec is wired correctly: encode a known picture, decode it, and check
        // the geometry and the light/dark halves come back.
        let (w, h) = (320u32, 240u32);
        let settings = StreamSettings {
            width: w,
            height: h,
            fps: 15,
            kbps: 1_000,
        };
        let mut encoder = Encoder::new(settings).expect("encoder");
        let mut decoder = Decoder::new().expect("decoder");
        let frame = test_frame(w, h);

        // A couple of frames: the first carries parameter sets, so a decoder may need both.
        let mut decoded = None;
        for _ in 0..3 {
            let packet = encoder.encode(&frame).expect("encode");
            assert!(!packet.is_empty(), "the encoder produced no data");
            if let Some(out) = decoder.decode(&packet).expect("decode") {
                decoded = Some(out);
            }
        }

        let (pixels, dw, dh) = decoded.expect("no frame decoded from three packets");
        assert_eq!((dw, dh), (w, h), "decoded size must match");
        assert_eq!(pixels.len(), (w * h * 4) as usize);

        // Sample the middle row on each side. H.264 is lossy, so allow generous tolerance; the point
        // is that dark stayed dark and light stayed light rather than the channels being scrambled.
        let row = (h / 2) as usize * w as usize * 4;
        let left = pixels[row + 4 * 20];
        let right = pixels[row + 4 * (w as usize - 20)];
        assert!(left < 90, "left half should stay dark, got {left}");
        assert!(right > 160, "right half should stay light, got {right}");
    }

    #[test]
    fn the_decoder_reports_opaque_pixels() {
        let (w, h) = (320u32, 240u32);
        let settings = StreamSettings {
            width: w,
            height: h,
            fps: 15,
            kbps: 800,
        };
        let mut encoder = Encoder::new(settings).expect("encoder");
        let mut decoder = Decoder::new().expect("decoder");
        let frame = test_frame(w, h);
        for _ in 0..3 {
            let packet = encoder.encode(&frame).expect("encode");
            if let Some((pixels, _, _)) = decoder.decode(&packet).expect("decode") {
                assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                return;
            }
        }
        panic!("nothing decoded");
    }

    #[test]
    fn encoding_a_moving_picture_costs_far_less_than_the_first_frame() {
        // The whole reason for H.264 over JPEG: only the change is sent. A static screen should
        // produce tiny packets after the first keyframe.
        let (w, h) = (320u32, 240u32);
        let mut encoder = Encoder::new(StreamSettings {
            width: w,
            height: h,
            fps: 15,
            kbps: 1_000,
        })
        .expect("encoder");
        let frame = test_frame(w, h);
        let first = encoder.encode(&frame).expect("first").len();
        let mut later = 0;
        for _ in 0..5 {
            later = encoder.encode(&frame).expect("later").len();
        }
        assert!(
            later * 4 < first,
            "an unchanged frame ({later} bytes) should be far smaller than the keyframe ({first})"
        );
    }
}
