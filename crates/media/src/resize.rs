//! Resizing a captured frame to an exact target size.
//!
//! # Why area averaging
//!
//! Recording a 2560×1440 screen as 1920×1080 is a 0.75 scale — not a whole-number ratio, so the
//! cheap "take every Nth pixel" approach cannot even express it, and where it can, it throws away
//! three quarters of the picture and keeps whatever happened to land on the sample point. On text
//! and thin UI lines that is the worst possible choice: strokes flicker between frames and fine
//! detail turns to noise.
//!
//! [`area_average`] instead gives every destination pixel the **average of the exact source
//! rectangle it covers**, including partial coverage at the edges. That is the standard choice for
//! shrinking an image (it is what "INTER_AREA" means in OpenCV): it is a proper low-pass filter, so
//! aliasing is removed rather than baked in, and it costs one pass over the source.
//!
//! The tests below pin the property that matters: a fine checkerboard must shrink to flat grey, not
//! to all-black or all-white.
//!
//! `ponytail:` for *enlarging*, area averaging degenerates to nearest-neighbour. We never enlarge —
//! a recording is only ever the same size or smaller — so bilinear/Lanczos would be dead code.

/// Bytes per pixel in the BGRA frames the capture path produces.
const CHANNELS: usize = 4;

/// Scales `src` (BGRA, `src_width` × `src_height`) to exactly `dst_width` × `dst_height`.
///
/// Returns an empty vector if any dimension is zero or `src` is too short for the stated size, so a
/// malformed frame can never panic a recording thread.
#[must_use]
pub fn area_average(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
) -> Vec<u8> {
    let (sw, sh) = (src_width as usize, src_height as usize);
    let (dw, dh) = (dst_width as usize, dst_height as usize);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh * CHANNELS {
        return Vec::new();
    }
    if sw == dw && sh == dh {
        return src[..sw * sh * CHANNELS].to_vec(); // nothing to do
    }

    let mut out = vec![0u8; dw * dh * CHANNELS];
    // Scale factors in source pixels per destination pixel.
    let x_ratio = sw as f64 / dw as f64;
    let y_ratio = sh as f64 / dh as f64;

    for dy in 0..dh {
        // The exact source band this output row covers.
        let y0 = dy as f64 * y_ratio;
        let y1 = (dy + 1) as f64 * y_ratio;
        let first_row = y0.floor() as usize;
        let last_row = ((y1.ceil() as usize).min(sh)).max(first_row + 1);

        for dx in 0..dw {
            let x0 = dx as f64 * x_ratio;
            let x1 = (dx + 1) as f64 * x_ratio;
            let first_col = x0.floor() as usize;
            let last_col = ((x1.ceil() as usize).min(sw)).max(first_col + 1);

            let mut sums = [0f64; CHANNELS];
            let mut total = 0f64;
            for sy in first_row..last_row.min(sh) {
                // How much of this source row falls inside the band.
                let row_weight = overlap(y0, y1, sy as f64);
                if row_weight <= 0.0 {
                    continue;
                }
                let row_start = sy * sw * CHANNELS;
                for sx in first_col..last_col.min(sw) {
                    let weight = row_weight * overlap(x0, x1, sx as f64);
                    if weight <= 0.0 {
                        continue;
                    }
                    let i = row_start + sx * CHANNELS;
                    for (channel, sum) in sums.iter_mut().enumerate() {
                        *sum += f64::from(src[i + channel]) * weight;
                    }
                    total += weight;
                }
            }
            let out_index = (dy * dw + dx) * CHANNELS;
            if total > 0.0 {
                for (channel, sum) in sums.iter().enumerate() {
                    // Round rather than truncate: truncation darkens every frame very slightly.
                    out[out_index + channel] = (sum / total).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
}

/// How much of the unit-wide source pixel starting at `pixel` lies inside `[start, end)`.
fn overlap(start: f64, end: f64, pixel: f64) -> f64 {
    (end.min(pixel + 1.0) - start.max(pixel)).max(0.0)
}

/// The largest size that fits inside `max_width` × `max_height` keeping the source aspect ratio.
///
/// Never enlarges: a recording asked to be bigger than the screen stays screen-sized, because
/// inventing pixels wastes space and helps nobody.
#[must_use]
pub fn fit_within(src_width: u32, src_height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if src_width == 0 || src_height == 0 || max_width == 0 || max_height == 0 {
        return (src_width.max(1), src_height.max(1));
    }
    if src_width <= max_width && src_height <= max_height {
        return (src_width, src_height);
    }
    let scale = (f64::from(max_width) / f64::from(src_width))
        .min(f64::from(max_height) / f64::from(src_height));
    let width = ((f64::from(src_width) * scale).round() as u32).max(1);
    let height = ((f64::from(src_height) * scale).round() as u32).max(1);
    // JPEG and most encoders prefer even dimensions; rounding down by one is invisible.
    (width & !1, height & !1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a BGRA image from a per-pixel grey value.
    fn grey_image(width: u32, height: u32, value: impl Fn(u32, u32) -> u8) -> Vec<u8> {
        let mut out = Vec::with_capacity((width * height) as usize * CHANNELS);
        for y in 0..height {
            for x in 0..width {
                let v = value(x, y);
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
        out
    }

    #[test]
    fn shrinking_two_by_two_to_one_pixel_averages_all_four() {
        let src = grey_image(2, 2, |x, y| if (x + y) % 2 == 0 { 0 } else { 200 });
        let out = area_average(&src, 2, 2, 1, 1);
        assert_eq!(out.len(), CHANNELS);
        assert_eq!(out[0], 100, "average of 0,200,200,0");
    }

    #[test]
    fn a_fine_checkerboard_becomes_flat_grey_rather_than_aliasing() {
        // The whole reason for area averaging. Point sampling would return all black or all white
        // depending on which pixel it happened to land on; averaging returns the true mean.
        let src = grey_image(64, 64, |x, y| if (x + y) % 2 == 0 { 0 } else { 255 });
        let out = area_average(&src, 64, 64, 8, 8);
        assert_eq!(out.len(), 8 * 8 * CHANNELS);
        for pixel in out.chunks_exact(CHANNELS) {
            assert!(
                (126..=129).contains(&pixel[0]),
                "expected mid grey, got {}",
                pixel[0]
            );
        }
    }

    #[test]
    fn the_exact_target_size_is_produced_even_for_a_fractional_ratio() {
        // 2560x1440 -> 1920x1080 is 0.75: the case an integer-step resizer cannot express at all.
        let src = grey_image(256, 144, |_, _| 128);
        let out = area_average(&src, 256, 144, 192, 108);
        assert_eq!(out.len(), 192 * 108 * CHANNELS);
        assert!(
            out.chunks_exact(CHANNELS).all(|p| p[0] == 128),
            "a flat image must stay flat"
        );
    }

    #[test]
    fn the_alpha_channel_survives() {
        let src = grey_image(4, 4, |_, _| 10);
        let out = area_average(&src, 4, 4, 2, 2);
        assert!(out.chunks_exact(CHANNELS).all(|p| p[3] == 255));
    }

    #[test]
    fn a_same_size_request_returns_the_frame_unchanged() {
        let src = grey_image(8, 8, |x, _| x as u8);
        assert_eq!(area_average(&src, 8, 8, 8, 8), src);
    }

    #[test]
    fn a_malformed_frame_returns_nothing_instead_of_panicking() {
        assert!(area_average(&[1, 2, 3], 100, 100, 10, 10).is_empty());
        assert!(area_average(&[], 0, 0, 10, 10).is_empty());
        assert!(area_average(&[0; 64], 4, 4, 0, 10).is_empty());
    }

    #[test]
    fn a_gradient_keeps_its_shape_when_halved() {
        // Left half dark, right half light: after halving the same split must still be there.
        let src = grey_image(8, 2, |x, _| if x < 4 { 0 } else { 200 });
        let out = area_average(&src, 8, 2, 4, 1);
        let values: Vec<u8> = out.chunks_exact(CHANNELS).map(|p| p[0]).collect();
        assert_eq!(values, vec![0, 0, 200, 200]);
    }

    #[test]
    fn fit_within_keeps_the_aspect_ratio_of_a_16_by_9_screen() {
        assert_eq!(fit_within(2560, 1440, 1920, 1080), (1920, 1080));
        assert_eq!(fit_within(2560, 1440, 1280, 1280), (1280, 720));
        assert_eq!(fit_within(1920, 1200, 1280, 720), (1152, 720));
    }

    #[test]
    fn fit_within_never_enlarges() {
        assert_eq!(fit_within(1280, 720, 3840, 2160), (1280, 720));
    }

    #[test]
    fn fit_within_returns_even_dimensions_for_encoders() {
        let (w, h) = fit_within(1919, 1079, 1000, 1000);
        assert_eq!(w % 2, 0, "width {w} must be even");
        assert_eq!(h % 2, 0, "height {h} must be even");
    }

    #[test]
    fn fit_within_survives_nonsense_input() {
        assert_eq!(fit_within(0, 0, 100, 100), (1, 1));
        assert_eq!(fit_within(100, 100, 0, 0), (100, 100));
    }
}
