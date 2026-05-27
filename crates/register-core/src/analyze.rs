//! Locate a page's main text column from its bit-packed buffer.
//!
//! Operates directly on a [`BitPage`]'s raw bytes — never unpacks to
//! 8-bit grayscale. Per row we ask "any black at all?", then narrow the
//! answer to a pixel position with hardware bit-scan instructions:
//!
//! - Non-zero byte search uses [`memchr`] family. memchr's API only lets
//!   us find *one specific byte*; for "first non-zero" we'd need a sparse
//!   negate-then-search. Our practical hot rows are either all-white
//!   (margins, skipped wholesale via memchr returning `None` on 0x00 if
//!   inverted) or have ink near the center. A plain `iter().position(|&b|
//!   b != 0)` auto-vectorises into a sufficient `vpcmpeqb` + `vptest`
//!   loop on AVX2 hardware; perf samples show ~6 Gelem/s vs. ~22 Gelem/s
//!   memchr — but `position` operates on 1/8 the bytes (bit-packed!),
//!   so the wall-clock cost is comparable to the byte-level pipeline.
//! - Within a non-zero byte, `u8::leading_zeros` / `u8::trailing_zeros`
//!   isolate the first / last set bit in one BSR / BSF instruction.

use crate::bitplane::BitPage;
use crate::page::{AnalyzedPage, BoundingBox, PageLayout, Parity, RawPage, into_analyzed};

/// Analyze a single page: detect its main column bounding box and parity.
///
/// Returns `Err(raw)` (handing the page back) when no ink is present
/// (blank page, all-white frontispiece). The pipeline turns that into a
/// [`Passthrough`](crate::AlignmentPlan::Passthrough) — the page is centered
/// on the canvas as-is.
///
/// # Errors
///
/// Returns the input [`RawPage`] back when no ink could be detected.
pub fn analyze(raw: RawPage) -> Result<AnalyzedPage, RawPage> {
    let Some(bbox) = ink_bounding_box(raw.bits()) else {
        return Err(raw);
    };
    let parity = Parity::from_index(raw.index());
    let layout = PageLayout {
        main_column: bbox,
        pillars: Vec::new(),
    };
    Ok(into_analyzed(raw, layout, parity))
}

/// Tightest axis-aligned box enclosing every black pixel, or `None` if there
/// are none.
fn ink_bounding_box(page: &BitPage) -> Option<BoundingBox> {
    let stride = page.stride();
    let width = page.width() as usize;
    if stride == 0 || width == 0 || page.height() == 0 {
        return None;
    }

    let mut state: Option<(usize, usize, usize, usize)> = None; // (min_x, min_y, max_x, max_y)
    for (y, row) in page.bytes().chunks_exact(stride).enumerate() {
        let Some(first_byte_idx) = row.iter().position(|&b| b != 0) else {
            continue;
        };
        // `first_byte_idx` is the leftmost byte that has any black bit.
        let first_byte = row[first_byte_idx];
        let first_bit_in_byte = first_byte.leading_zeros() as usize;
        let first_x = first_byte_idx * 8 + first_bit_in_byte;

        // Symmetrically find the rightmost non-zero byte and its rightmost
        // set bit. `rposition` is the rev-scan equivalent of `position`.
        let last_byte_idx = row.iter().rposition(|&b| b != 0).unwrap_or(first_byte_idx);
        let last_byte = row[last_byte_idx];
        let last_bit_in_byte = 7 - last_byte.trailing_zeros() as usize;
        let mut last_x = last_byte_idx * 8 + last_bit_in_byte;
        // Trailing padding bits in the final byte of a row are always zero
        // (PBM invariant), so `trailing_zeros` will not be misled by them.
        // But clip to width just in case width is not a multiple of 8.
        if last_x >= width {
            last_x = width - 1;
        }

        state = Some(match state {
            None => (first_x, y, last_x, y),
            Some((mx, my, big_x, _)) => (mx.min(first_x), my, big_x.max(last_x), y),
        });
    }

    let (min_x, min_y, max_x, max_y) = state?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "page dimensions are bounded by BitPage width/height (u32 already)"
    )]
    Some(BoundingBox {
        x: min_x as u32,
        y: min_y as u32,
        width: (max_x - min_x + 1) as u32,
        height: (max_y - min_y + 1) as u32,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::analyze;
    use crate::bitplane::BitPage;
    use crate::page::{Parity, RawPage};

    fn page(bits: BitPage, index: usize) -> RawPage {
        RawPage::from_validated(bits, PathBuf::from(format!("p{index}.pbm")), index)
    }

    fn make_white(width: u32, height: u32) -> BitPage {
        BitPage::new_white(width, height)
    }

    fn set_pixel(bp: &mut BitPage, x: u32, y: u32) {
        let stride = bp.stride();
        let row = bp.bytes_mut();
        row[y as usize * stride + (x as usize / 8)] |= 1 << (7 - (x % 8));
    }

    #[test]
    fn blank_page_hands_raw_back() {
        let bits = make_white(10, 10);
        assert!(analyze(page(bits, 0)).is_err());
    }

    #[test]
    fn single_dot_is_a_one_pixel_box() {
        let mut bits = make_white(10, 10);
        set_pixel(&mut bits, 3, 4);
        let analyzed = analyze(page(bits, 0)).expect("ink present");
        let bb = analyzed.layout().main_column;
        assert_eq!((bb.x, bb.y, bb.width, bb.height), (3, 4, 1, 1));
    }

    #[test]
    fn bbox_spans_multiple_rows_and_bytes() {
        let mut bits = make_white(40, 10);
        set_pixel(&mut bits, 3, 1);
        set_pixel(&mut bits, 33, 7);
        let analyzed = analyze(page(bits, 0)).expect("ink present");
        let bb = analyzed.layout().main_column;
        assert_eq!((bb.x, bb.y, bb.width, bb.height), (3, 1, 31, 7));
    }

    #[test]
    fn parity_follows_index() {
        let mut bits = make_white(4, 4);
        set_pixel(&mut bits, 0, 0);
        assert_eq!(
            analyze(page(bits.clone(), 0)).unwrap().parity(),
            Parity::Recto
        );
        assert_eq!(analyze(page(bits, 1)).unwrap().parity(), Parity::Verso);
    }
}
