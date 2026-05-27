//! Render an [`AlignmentPlan`] onto the paper-size canvas.
//!
//! Two paths:
//!
//! - **Translate / Passthrough**: both end in [`blit_lossless_bits`], a
//!   per-row `copy_from_slice` of the source bit buffer into the canvas.
//!   `dx` is rounded to a multiple of 8 in [`crate::plan`] so the copy is
//!   byte-aligned and stays a pure memcpy. The result is bit-exact and
//!   ~8× cheaper than the previous byte-per-pixel pipeline.
//!
//! - **Affine**: scale / skew / sub-pixel translation cannot be done
//!   losslessly in 1-bit. The plan up-converts to grayscale, applies the
//!   bilinear warp via `imageproc`, re-binarises, and packs back into the
//!   canvas's [`BitPage`]. This is the only site that touches grayscale —
//!   the rest of the pipeline never pays the 8× memory cost.

use image::{GrayImage, Luma};
use imageproc::geometric_transformations::{Interpolation, warp_into};

use crate::bitplane::BitPage;
use crate::page::{AlignmentKind, AlignmentPlan, RawPage, RegisteredPage};
use crate::paper::TargetCanvas;

const WHITE_GRAY: Luma<u8> = Luma([255]);
const BLACK_THRESHOLD: u8 = 128;

/// A re-usable destination canvas for [`render_into`].
///
/// `CanvasBuf` owns the page-sized bit buffer (≈ 815 KiB at B5/300 DPI vs.
/// the 6.5 MiB the previous byte-per-pixel buffer needed). Allocating a
/// fresh one per page costs more than the actual rendering for a tight
/// pipeline; rent one per rayon worker thread and reuse it.
pub struct CanvasBuf {
    bits: BitPage,
}

impl CanvasBuf {
    /// Allocate a fresh, white buffer sized to `canvas`.
    #[must_use]
    pub fn new(canvas: TargetCanvas) -> Self {
        Self {
            bits: BitPage::new_white(canvas.width, canvas.height),
        }
    }

    /// Reset every byte to `0` (white), ready for the next page.
    pub fn reset(&mut self) {
        self.bits.reset_white();
    }

    /// Borrow the underlying bit-packed buffer.
    #[must_use]
    pub const fn bits(&self) -> &BitPage {
        &self.bits
    }
}

/// Apply an [`AlignmentPlan`] to its source page and return a registered
/// page sized exactly to `canvas`.
///
/// Allocates a fresh output buffer. For corpus runs prefer [`render_into`]
/// + [`CanvasBuf`] so that the per-page allocation amortizes across the
/// worker's lifetime.
#[must_use]
pub fn render(plan: AlignmentPlan, canvas: TargetCanvas) -> RegisteredPage {
    let source = plan.source().to_path_buf();
    let mut buf = CanvasBuf::new(canvas);
    render_into(plan, canvas, &mut buf);
    RegisteredPage {
        bits: buf.bits,
        source,
    }
}

/// Render `plan` into a pre-allocated [`CanvasBuf`].
///
/// The buffer is reset to white before the render begins, so the caller can
/// safely re-use the same `CanvasBuf` across many pages without having to
/// remember to clean up.
pub fn render_into(plan: AlignmentPlan, canvas: TargetCanvas, buf: &mut CanvasBuf) {
    buf.reset();
    match plan {
        AlignmentPlan::Aligned {
            analyzed,
            kind: AlignmentKind::Translate { dx, dy },
        } => {
            // Plan rounds `dx` to a multiple of 8 so the copy stays byte-aligned.
            blit_lossless_bits(analyzed.raw.bits(), &mut buf.bits, dx, dy);
        },
        AlignmentPlan::Aligned {
            analyzed,
            kind: AlignmentKind::Affine(transform),
        } => {
            // Affine path: up-convert to grayscale, bilinear warp, re-binarise,
            // pack back into the canvas. Only triggered by scale/skew components.
            render_affine_into(&analyzed.raw, &transform, canvas, &mut buf.bits);
        },
        AlignmentPlan::Passthrough { raw } => {
            // Center the source on the canvas (offset rounded down to a
            // byte boundary on the x axis to keep the copy a pure memcpy).
            let (src_w, src_h) = (raw.bits.width(), raw.bits.height());
            let dx_pixels = round_down_to_byte((i64::from(canvas.width) - i64::from(src_w)) / 2);
            let dy_pixels = (i64::from(canvas.height) - i64::from(src_h)) / 2;
            blit_lossless_bits(&raw.bits, &mut buf.bits, dx_pixels as i32, dy_pixels as i32);
        },
    }
}

/// Copy `src` into `dest` shifted by integer pixels `(dx, dy)`. `dx` is
/// rounded toward `-∞` to the nearest multiple of 8 so the copy stays
/// byte-aligned (and thus a pure memcpy). Values not already aligned shift
/// by ≤ 7 source-pixels — invisible at 300 DPI on bitonal text.
///
/// Clipped automatically: rows that would land off the canvas are dropped,
/// and partial rows are byte-truncated (≤ 7 pixels lost at the right edge
/// when the canvas's right edge isn't byte-aligned).
fn blit_lossless_bits(src: &BitPage, dest: &mut BitPage, dx: i32, dy: i32) {
    let src_w = i64::from(src.width());
    let src_h = i64::from(src.height());
    let dest_w = i64::from(dest.width());
    let dest_h = i64::from(dest.height());

    let dx_aligned = round_down_to_byte(i64::from(dx));
    let dy_i = i64::from(dy);

    // Source y range that maps into dest: dy + sy ∈ [0, dest_h).
    let sy0 = (-dy_i).max(0);
    let sy1 = (dest_h - dy_i).min(src_h);
    if sy0 >= sy1 {
        return;
    }
    // Source x range, byte-aligned (sx0 is multiple of 8 because dx_aligned is).
    let sx0 = (-dx_aligned).max(0);
    let sx1_pixels = (dest_w - dx_aligned).min(src_w);
    // Truncate sx1 to a multiple of 8 so the per-row copy ends at a byte
    // boundary. We may drop ≤ 7 source pixels on the right; for B5 novel
    // pages this is well below the visible threshold.
    let sx1 = (sx1_pixels / 8) * 8;
    if sx0 >= sx1 {
        return;
    }

    #[expect(clippy::cast_sign_loss, reason = "sx0/sx1 clipped >= 0 above")]
    let sx0_byte = (sx0 / 8) as usize;
    #[expect(clippy::cast_sign_loss, reason = "sx0/sx1 clipped >= 0 above")]
    let span_bytes = ((sx1 - sx0) / 8) as usize;
    if span_bytes == 0 {
        return;
    }
    #[expect(
        clippy::cast_sign_loss,
        reason = "sx0 + dx_aligned >= 0 by construction"
    )]
    let dest_x_byte = ((sx0 + dx_aligned) / 8) as usize;

    let src_stride = src.stride();
    let dest_stride = dest.stride();
    let src_bytes = src.bytes();
    let dest_bytes = dest.bytes_mut();

    for sy in sy0..sy1 {
        #[expect(clippy::cast_sign_loss, reason = "sy in [0, src_h)")]
        let sy_us = sy as usize;
        #[expect(clippy::cast_sign_loss, reason = "sy + dy_i in [0, dest_h)")]
        let dy_us = (sy + dy_i) as usize;
        let src_off = sy_us * src_stride + sx0_byte;
        let dest_off = dy_us * dest_stride + dest_x_byte;
        dest_bytes[dest_off..dest_off + span_bytes]
            .copy_from_slice(&src_bytes[src_off..src_off + span_bytes]);
    }
}

#[inline]
const fn round_down_to_byte(x: i64) -> i64 {
    // Round toward negative infinity to nearest multiple of 8. Integer
    // division in Rust truncates toward zero, so for negatives we use
    // `(x - 7) / 8 * 8` which rounds toward -∞ for non-multiples.
    if x >= 0 {
        (x / 8) * 8
    } else {
        ((x - 7) / 8) * 8
    }
}

/// Affine fallback: BitPage → GrayImage → warp_into → re-binarise → BitPage.
fn render_affine_into(
    raw: &RawPage,
    transform: &imageproc::geometric_transformations::Projection,
    canvas: TargetCanvas,
    dest: &mut BitPage,
) {
    let src_gray = bit_unpack_to_grayscale(&raw.bits);
    let mut tmp = GrayImage::from_pixel(canvas.width, canvas.height, WHITE_GRAY);
    warp_into(
        &src_gray,
        transform,
        Interpolation::Nearest,
        WHITE_GRAY,
        &mut tmp,
    );
    rebinarise_in_place(&mut tmp);
    pack_into_bitpage(&tmp, dest);
}

fn rebinarise_in_place(img: &mut GrayImage) {
    for px in img.pixels_mut() {
        px.0[0] = if px.0[0] < BLACK_THRESHOLD { 0 } else { 255 };
    }
}

pub(crate) fn bit_unpack_to_grayscale(bp: &BitPage) -> GrayImage {
    let width = bp.width();
    let height = bp.height();
    let stride = bp.stride();
    let mut out = vec![255_u8; (width * height) as usize];
    let table = crate::bitplane::unpack_table();
    let full_bytes_per_row = (width as usize) / 8;
    let tail_bits = width as usize - full_bytes_per_row * 8;
    for y in 0..height as usize {
        let src_row = &bp.bytes()[y * stride..(y + 1) * stride];
        let dest_row = &mut out[y * width as usize..(y + 1) * width as usize];
        let mut dx = 0;
        for &sb in &src_row[..full_bytes_per_row] {
            dest_row[dx..dx + 8].copy_from_slice(&table[sb as usize]);
            dx += 8;
        }
        if tail_bits > 0 {
            let sb = src_row[full_bytes_per_row];
            dest_row[dx..dx + tail_bits].copy_from_slice(&table[sb as usize][..tail_bits]);
        }
    }
    // `out` is exactly `width * height` bytes by construction above, so
    // `from_raw` cannot fail; the `unwrap_or_else` arm is dead.
    GrayImage::from_raw(width, height, out).unwrap_or_else(|| GrayImage::new(width, height))
}

fn pack_into_bitpage(gray: &GrayImage, dest: &mut BitPage) {
    debug_assert_eq!(gray.width(), dest.width());
    debug_assert_eq!(gray.height(), dest.height());
    let width = gray.width() as usize;
    let height = gray.height() as usize;
    let stride = dest.stride();
    let src = gray.as_raw();
    for y in 0..height {
        let src_row = &src[y * width..(y + 1) * width];
        let dest_row = &mut dest.bytes_mut()[y * stride..(y + 1) * stride];
        crate::bitplane::pack_row(src_row, dest_row, width);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use imageproc::geometric_transformations::Projection;

    use super::{CanvasBuf, render, render_into};
    use crate::bitplane::BitPage;
    use crate::page::{
        AlignmentKind, AlignmentPlan, BoundingBox, PageLayout, Parity, RawPage, into_analyzed,
    };
    use crate::paper::TargetCanvas;

    fn make_white(width: u32, height: u32) -> BitPage {
        BitPage::new_white(width, height)
    }

    fn set_pixel(bp: &mut BitPage, x: u32, y: u32) {
        let stride = bp.stride();
        let row = bp.bytes_mut();
        row[y as usize * stride + (x as usize / 8)] |= 1 << (7 - (x % 8));
    }

    fn get_pixel(bp: &BitPage, x: u32, y: u32) -> u8 {
        let stride = bp.stride();
        let byte = bp.bytes()[y as usize * stride + (x as usize / 8)];
        (byte >> (7 - (x % 8))) & 1
    }

    #[test]
    fn passthrough_centers_smaller_source_on_canvas() {
        let mut bits = make_white(8, 8);
        set_pixel(&mut bits, 0, 0);
        let raw = RawPage::from_validated(bits, PathBuf::from("p.pbm"), 0);
        let canvas = TargetCanvas {
            width: 32,
            height: 32,
            dpi: 100,
        };
        let plan = AlignmentPlan::Passthrough { raw };
        let registered = render(plan, canvas);
        assert_eq!(registered.bits().width(), 32);
        assert_eq!(registered.bits().height(), 32);
        // Source 8×8 on canvas 32×32: dx-unaligned = (32-8)/2 = 12, rounded
        // down to multiple of 8 = 8. dy = 12 (no x-alignment constraint on y).
        // So source (0,0) lands at canvas (8, 12).
        assert_eq!(get_pixel(registered.bits(), 8, 12), 1);
    }

    #[test]
    fn aligned_translate_lands_on_target_pixel() {
        let mut bits = make_white(16, 16);
        set_pixel(&mut bits, 0, 0);
        let raw = RawPage::from_validated(bits, PathBuf::from("p.pbm"), 0);
        let layout = PageLayout {
            main_column: BoundingBox {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            pillars: Vec::new(),
        };
        let analyzed = into_analyzed(raw, layout, Parity::Recto);
        let canvas = TargetCanvas {
            width: 32,
            height: 32,
            dpi: 100,
        };
        // Translate by (8, 5): source (0,0) → canvas (8, 5).
        let plan = AlignmentPlan::Aligned {
            analyzed,
            kind: AlignmentKind::Translate { dx: 8, dy: 5 },
        };
        let registered = render(plan, canvas);
        assert_eq!(get_pixel(registered.bits(), 8, 5), 1);
    }

    #[test]
    fn render_into_reuses_canvas_buf() {
        let mut bits = make_white(16, 16);
        set_pixel(&mut bits, 0, 0);
        let raw = RawPage::from_validated(bits, PathBuf::from("p.pbm"), 0);
        let layout = PageLayout {
            main_column: BoundingBox {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            pillars: Vec::new(),
        };
        let analyzed = into_analyzed(raw, layout, Parity::Recto);
        let canvas = TargetCanvas {
            width: 32,
            height: 32,
            dpi: 100,
        };
        let mut buf = CanvasBuf::new(canvas);
        let plan = AlignmentPlan::Aligned {
            analyzed,
            kind: AlignmentKind::Translate { dx: 8, dy: 5 },
        };
        render_into(plan, canvas, &mut buf);
        assert_eq!(get_pixel(buf.bits(), 8, 5), 1);
    }

    #[test]
    fn aligned_affine_falls_through_to_warp() {
        let mut bits = make_white(16, 16);
        set_pixel(&mut bits, 5, 5);
        let raw = RawPage::from_validated(bits, PathBuf::from("p.pbm"), 0);
        let layout = PageLayout {
            main_column: BoundingBox {
                x: 5,
                y: 5,
                width: 1,
                height: 1,
            },
            pillars: Vec::new(),
        };
        let analyzed = into_analyzed(raw, layout, Parity::Recto);
        let canvas = TargetCanvas {
            width: 32,
            height: 32,
            dpi: 100,
        };
        let projection = Projection::translate(10.0, 10.0);
        let plan = AlignmentPlan::Aligned {
            analyzed,
            kind: AlignmentKind::Affine(projection),
        };
        let registered = render(plan, canvas);
        // (5, 5) shifted by (10, 10) → (15, 15).
        assert_eq!(get_pixel(registered.bits(), 15, 15), 1);
    }
}
