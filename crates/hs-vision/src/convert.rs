//! Pixel format conversion helpers.
//!
//! The Linux capture backend negotiates packed YUYV (4:2:2) frames. Only the
//! luminance channel is needed by the detector, so the conversion simply picks
//! the first byte of every two-pixel macro-pixel. Keeping this pure (no I/O, no
//! `unsafe`) means it is unit tested on every platform, including Windows CI.

use crate::{VisionError, VisionResult};

/// Convert a packed YUYV (4:2:2) capture buffer to a grayscale luma plane.
///
/// `stride` is the number of bytes per row as reported by the driver; it is
/// usually `2 * width` but drivers may pad rows. `data` must contain at least
/// `stride * height` bytes.
///
/// # Errors
///
/// Returns [`VisionError::InvalidFrame`] for zero dimensions, an undersized
/// stride/buffer or inconsistent arguments.
// The only caller is the Linux capture backend; the conversion itself is unit
// tested on every platform, so silence the dead-code lint on non-Linux builds.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn yuyv_to_luma(
    width: u32,
    height: u32,
    stride: usize,
    data: &[u8],
) -> VisionResult<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    if width == 0 || height == 0 {
        return Err(VisionError::InvalidFrame(
            "cannot convert a zero-sized frame".to_string(),
        ));
    }
    let row_bytes = width
        .checked_mul(2)
        .ok_or_else(|| VisionError::InvalidFrame("row size overflow".to_string()))?;
    if stride < row_bytes {
        return Err(VisionError::InvalidFrame(format!(
            "YUYV stride {stride} is smaller than the {row_bytes} bytes required for {width} pixels"
        )));
    }
    let required = stride
        .checked_mul(height)
        .ok_or_else(|| VisionError::InvalidFrame("frame size overflow".to_string()))?;
    if data.len() < required {
        return Err(VisionError::InvalidFrame(format!(
            "YUYV buffer holds {} bytes but {required} are required",
            data.len()
        )));
    }

    let mut luma = vec![0u8; width.saturating_mul(height)];
    let rows = data.chunks_exact(stride).take(height);
    for (row, out) in rows.zip(luma.chunks_exact_mut(width)) {
        for (x, pixel) in out.iter_mut().enumerate() {
            *pixel = row[2 * x];
        }
    }
    Ok(luma)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_packed_yuyv_to_luma() {
        // Two pixels packed as Y0 U Y1 V.
        let data = [10u8, 20, 30, 40];
        let luma = yuyv_to_luma(2, 1, 4, &data).expect("conversion");
        assert_eq!(luma, vec![10, 30]);
    }

    #[test]
    fn handles_odd_width_and_row_padding() {
        // Width 3 means two YUYV macro-pixels (6 bytes) per row; stride is
        // padded to 8 bytes.
        let row_one = [10u8, 20, 30, 40, 50, 60, 0, 0];
        let row_two = [70u8, 80, 90, 100, 110, 120, 0, 0];
        let mut data = Vec::new();
        data.extend_from_slice(&row_one);
        data.extend_from_slice(&row_two);
        let luma = yuyv_to_luma(3, 2, 8, &data).expect("conversion");
        assert_eq!(luma, vec![10, 30, 50, 70, 90, 110]);
    }

    #[test]
    fn rejects_short_buffers() {
        let error = yuyv_to_luma(2, 2, 4, &[0; 7]).expect_err("short buffer must fail");
        assert!(matches!(error, VisionError::InvalidFrame(_)));
    }

    #[test]
    fn rejects_undersized_stride() {
        let error = yuyv_to_luma(2, 1, 3, &[0; 12]).expect_err("small stride must fail");
        assert!(matches!(error, VisionError::InvalidFrame(_)));
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert!(yuyv_to_luma(0, 1, 4, &[0; 4]).is_err());
        assert!(yuyv_to_luma(2, 0, 4, &[0; 4]).is_err());
    }
}
