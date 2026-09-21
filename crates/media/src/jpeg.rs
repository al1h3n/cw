//! Decoding JPEG back into pixels.
//!
//! Everything else in this crate *produces* JPEG. Broadcasting reverses the direction: a student PC
//! receives the teacher's screen as JPEG and has to put it on a window, which means turning it back
//! into the BGRA the Windows drawing calls want.
//!
//! `zune-jpeg` is a pure-Rust baseline decoder — no C, so it does not complicate the static-CRT
//! build, and no `unsafe` of ours.

use zune_jpeg::{JpegDecoder, zune_core::options::DecoderOptions};

use crate::CaptureError;

/// Decodes JPEG bytes into packed BGRA, returning `(pixels, width, height)`.
///
/// BGRA rather than RGB because that is what `StretchDIBits` and the capture path both use, so a
/// decoded broadcast frame goes straight to the screen with no second conversion.
///
/// # Errors
/// Returns [`CaptureError`] if the bytes are not a JPEG this decoder understands.
pub fn decode_to_bgra(jpeg: &[u8]) -> Result<(Vec<u8>, u32, u32), CaptureError> {
    // Ask for RGB explicitly rather than accepting whatever the file declares, so a greyscale or
    // CMYK JPEG still arrives as three bytes per pixel and the loop below stays correct.
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGB);
    // zune-jpeg 0.5 takes a reader implementing `ZByteReaderTrait` rather than a raw `&[u8]`;
    // `ZCursor` wraps the slice for that without copying the bytes.
    let reader = zune_jpeg::zune_core::bytestream::ZCursor::new(jpeg);
    let mut decoder = JpegDecoder::new_with_options(reader, options);
    decoder
        .decode_headers()
        .map_err(|e| CaptureError(format!("not a readable JPEG: {e}")))?;
    let (width, height) = decoder
        .dimensions()
        .ok_or_else(|| CaptureError("JPEG has no dimensions".into()))?;

    let rgb = decoder
        .decode()
        .map_err(|e| CaptureError(format!("JPEG decode failed: {e}")))?;

    let pixels = width * height;
    if rgb.len() < pixels * 3 {
        return Err(CaptureError("JPEG decoded short".into()));
    }
    let mut bgra = Vec::with_capacity(pixels * 4);
    for chunk in rgb.chunks_exact(3).take(pixels) {
        // RGB in, BGRA out.
        bgra.extend_from_slice(&[chunk[2], chunk[1], chunk[0], 255]);
    }
    Ok((
        bgra,
        u32::try_from(width).unwrap_or(0),
        u32::try_from(height).unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes a flat colour so the round trip has something predictable to check.
    fn flat_jpeg(width: u16, height: u16, bgr: [u8; 3]) -> Vec<u8> {
        let mut bgra = Vec::new();
        for _ in 0..(u32::from(width) * u32::from(height)) {
            bgra.extend_from_slice(&[bgr[0], bgr[1], bgr[2], 255]);
        }
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 90)
            .encode(&bgra, width, height, jpeg_encoder::ColorType::Bgra)
            .expect("encode");
        out
    }

    #[test]
    fn a_frame_survives_the_round_trip_at_the_right_size() {
        let jpeg = flat_jpeg(64, 32, [10, 20, 200]);
        let (pixels, width, height) = decode_to_bgra(&jpeg).expect("decode");
        assert_eq!((width, height), (64, 32));
        assert_eq!(pixels.len(), 64 * 32 * 4);
    }

    #[test]
    fn colours_come_back_in_bgra_order_not_rgba() {
        // A strong red: in BGRA the last colour byte is the big one.
        let jpeg = flat_jpeg(16, 16, [0, 0, 255]);
        let (pixels, _, _) = decode_to_bgra(&jpeg).expect("decode");
        let middle = (16 * 8 + 8) * 4;
        let pixel = &pixels[middle..middle + 4];
        assert!(pixel[2] > 200, "red channel should be high, got {pixel:?}");
        assert!(pixel[0] < 60, "blue channel should be low, got {pixel:?}");
        assert_eq!(pixel[3], 255, "alpha is opaque");
    }

    #[test]
    fn every_pixel_is_opaque_so_nothing_shows_through_a_broadcast() {
        let jpeg = flat_jpeg(8, 8, [90, 90, 90]);
        let (pixels, _, _) = decode_to_bgra(&jpeg).expect("decode");
        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn junk_is_refused_rather_than_panicking() {
        assert!(decode_to_bgra(&[]).is_err());
        assert!(decode_to_bgra(b"this is not a jpeg at all").is_err());
        // A valid header followed by nothing useful.
        assert!(decode_to_bgra(&[0xFF, 0xD8, 0xFF, 0xD9]).is_err());
    }
}
