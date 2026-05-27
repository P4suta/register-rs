//! Locate a page's main text column from its bit-packed buffer.
//!
//! The fundamental question is "where is the body text?", and the naive
//! answer — bbox of every ink pixel — is wrong for any non-trivial book:
//! ノンブル, 柱 (running titles), 段落番号 in the margin, footnotes,
//! marginalia of every kind contribute ink that's *not* part of the
//! main text block. Aligning by their bbox throws the body text off by
//! whatever the marginalia happen to be.
//!
//! The fix is the standard projection-profile approach:
//!
//! 1. Row projection: for every row y, count the ink pixels via popcount.
//!    Threshold and find the longest **contiguous dense band** (with small
//!    gaps bridged) — this excludes 柱 / ノンブル, which sit on a handful
//!    of rows separated from the body by ≥ several rows of whitespace.
//! 2. Column projection within that band: for every column x, count the
//!    rows in the band where x has ink. Threshold and find the longest
//!    contiguous dense band again — this excludes marginal numbers /
//!    footnote indices, whose columns have ink on only a small fraction
//!    of body rows.
//!
//! The intersection of those two bands is the main text block.

use crate::bitplane::BitPage;
use crate::page::{AnalyzedPage, BoundingBox, PageLayout, Parity, RawPage, into_analyzed};

/// A row/column qualifies as "dense" if its ink count is at least
/// `max(projection) / DENSITY_DENOM`. 1/8 is empirically the right trade-off
/// for Japanese novel scans: low enough to admit the sparser glyphs at the
/// edges of the text column without admitting the much sparser marginalia
/// (where the count is typically 1–10% of the main-column max).
const DENSITY_DENOM: u32 = 8;

/// Gaps shorter than `1 / GAP_DENOM` of the projection length get bridged
/// when finding the longest contiguous dense band. Sized so paragraph
/// breaks and chapter gaps inside the main text don't fragment it, while
/// the ≥ 50-row gap between 柱 and the body text stays unbridged.
const GAP_DENOM: usize = 32;

/// Analyze a single page: detect its main column bounding box and parity.
///
/// Returns `Err(raw)` (handing the page back) when no body text could be
/// located. The pipeline turns that into a
/// [`Passthrough`](crate::AlignmentPlan::Passthrough) — the page is centered
/// on the canvas as-is.
///
/// # Errors
///
/// Returns the input [`RawPage`] back when no main column could be
/// detected (blank page, frontispiece, image-only page).
pub fn analyze(raw: RawPage) -> Result<AnalyzedPage, RawPage> {
    let Some(bbox) = main_column_bbox(raw.bits()) else {
        return Err(raw);
    };
    let parity = Parity::from_index(raw.index());
    let layout = PageLayout {
        main_column: bbox,
        pillars: Vec::new(),
    };
    Ok(into_analyzed(raw, layout, parity))
}

/// Find the main text block via row + column projection profiles.
fn main_column_bbox(page: &BitPage) -> Option<BoundingBox> {
    let height = page.height() as usize;
    let width = page.width() as usize;
    let stride = page.stride();
    if height == 0 || width == 0 || stride == 0 {
        return None;
    }

    // --- Step 1: row projection (popcount per row) -----------------------
    let row_ink: Vec<u32> = (0..height)
        .map(|y| {
            page.bytes()[y * stride..(y + 1) * stride]
                .iter()
                .map(|b| b.count_ones())
                .sum()
        })
        .collect();
    let max_row = *row_ink.iter().max()?;
    if max_row == 0 {
        return None;
    }
    let row_thresh = (max_row / DENSITY_DENOM).max(1);
    let row_gap = (height / GAP_DENOM).max(2);
    let (y0, y1) = densest_band(&row_ink, row_thresh, row_gap)?;

    // --- Step 2: column projection within the dense row band ------------
    let col_ink = column_projection(page, y0, y1);
    let max_col = *col_ink.iter().max()?;
    if max_col == 0 {
        return None;
    }
    let col_thresh = (max_col / DENSITY_DENOM).max(1);
    let col_gap = (width / GAP_DENOM).max(2);
    let (x0, x1) = densest_band(&col_ink, col_thresh, col_gap)?;

    Some(BoundingBox {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    })
}

/// For each column x, count the rows in `[y0, y1)` where the bit at `x` is
/// set in `page`. Returns a `width`-long projection.
fn column_projection(page: &BitPage, y0: usize, y1: usize) -> Vec<u32> {
    let width = page.width() as usize;
    let stride = page.stride();
    let mut col_ink = vec![0_u32; width];
    for y in y0..y1 {
        let row = &page.bytes()[y * stride..(y + 1) * stride];
        for (byte_idx, &byte) in row.iter().enumerate() {
            if byte == 0 {
                continue;
            }
            let base = byte_idx * 8;
            // Walk set bits from the MSB (= leftmost pixel in the byte).
            let mut b = byte;
            while b != 0 {
                let bit = b.leading_zeros() as usize;
                let x = base + bit;
                if x < width {
                    col_ink[x] += 1;
                }
                // Clear that bit and keep going.
                b &= !(0x80_u8 >> bit);
            }
        }
    }
    col_ink
}

/// Longest contiguous "dense" band in a 1-D projection.
///
/// Positions with `projection[i] >= thresh` are dense; gaps of up to
/// `max_gap` consecutive sparse positions are bridged (so paragraph
/// whitespace doesn't fragment the body text). Returns `(start, end)` —
/// `end` is exclusive and trimmed to the last actually-dense position.
fn densest_band(projection: &[u32], thresh: u32, max_gap: usize) -> Option<(usize, usize)> {
    let n = projection.len();
    if n == 0 {
        return None;
    }

    let mut best: Option<(usize, usize)> = None;
    let mut run_start: Option<usize> = None;
    let mut last_dense: Option<usize> = None;

    let close_run = |best: &mut Option<(usize, usize)>, start: usize, last: usize| {
        let end = last + 1;
        let len = end - start;
        if best.is_none_or(|(s, e)| e - s < len) {
            *best = Some((start, end));
        }
    };

    for (i, &v) in projection.iter().enumerate() {
        if v >= thresh {
            run_start.get_or_insert(i);
            last_dense = Some(i);
        } else if let (Some(start), Some(prev_dense)) = (run_start, last_dense) {
            // We're in a gap; close the run only if the gap is too wide
            // to bridge.
            if i - prev_dense > max_gap {
                close_run(&mut best, start, prev_dense);
                run_start = None;
                last_dense = None;
            }
        }
    }
    if let (Some(start), Some(prev_dense)) = (run_start, last_dense) {
        close_run(&mut best, start, prev_dense);
    }

    best
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

    /// Fill the rectangle `[x0, x0+w) × [y0, y0+h)` with ink.
    fn fill_block(bp: &mut BitPage, x0: u32, y0: u32, w: u32, h: u32) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                set_pixel(bp, x, y);
            }
        }
    }

    #[test]
    fn blank_page_hands_raw_back() {
        let bits = make_white(100, 100);
        assert!(analyze(page(bits, 0)).is_err());
    }

    #[test]
    fn dense_rectangle_is_detected_exactly() {
        // A 200×400 page with a 60×200 dense block at (50, 100).
        let mut bits = make_white(200, 400);
        fill_block(&mut bits, 50, 100, 60, 200);
        let analyzed = analyze(page(bits, 0)).expect("dense block present");
        let bb = analyzed.layout().main_column;
        assert_eq!((bb.x, bb.y, bb.width, bb.height), (50, 100, 60, 200));
    }

    #[test]
    fn marginalia_column_is_excluded() {
        // Mimic the 省察 case: a dense main column plus a sparse strip of
        // ink in the left margin (the paragraph-number column). The
        // projection-based detector should pick up the main column only.
        let mut bits = make_white(200, 400);
        // Main column: dense block at (80, 50, 100, 300)
        fill_block(&mut bits, 80, 50, 100, 300);
        // Sparse paragraph-number column: 6 marks of size 8x6 at x=10,
        // separated by 40+ row gaps in the y direction (so each mark only
        // contributes ink for 6 rows — well below the column-density
        // threshold).
        for &y in &[60_u32, 110, 160, 210, 260, 310] {
            fill_block(&mut bits, 10, y, 8, 6);
        }
        let analyzed = analyze(page(bits, 0)).expect("dense block present");
        let bb = analyzed.layout().main_column;
        // Bbox should be the main column at (80, ..) — NOT (10, ..).
        assert!(
            bb.x >= 80 && bb.x + bb.width <= 180,
            "bbox {bb:?} should sit inside the main column [80, 180), not include x=10 numbers",
        );
    }

    #[test]
    fn header_row_is_excluded() {
        // Mimic 柱: a thin band of ink at the top of the page, separated
        // from the main column by a wide gap.
        let mut bits = make_white(200, 400);
        // 柱: 3 rows of ink at y=10..13
        fill_block(&mut bits, 50, 10, 100, 3);
        // Main column: dense block from y=80..380
        fill_block(&mut bits, 50, 80, 100, 300);
        let analyzed = analyze(page(bits, 0)).expect("dense block present");
        let bb = analyzed.layout().main_column;
        // Bbox should start at the main column (y >= 80), not at y=10.
        assert!(
            bb.y >= 50,
            "bbox {bb:?} should not include the 3-row 柱 band at y=10",
        );
    }

    #[test]
    fn parity_follows_index() {
        let mut bits = make_white(200, 400);
        fill_block(&mut bits, 50, 100, 100, 200);
        assert_eq!(
            analyze(page(bits.clone(), 0)).unwrap().parity(),
            Parity::Recto
        );
        assert_eq!(analyze(page(bits, 1)).unwrap().parity(), Parity::Verso);
    }
}
