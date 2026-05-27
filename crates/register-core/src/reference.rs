//! Derive the per-parity reference layout from the analyzed corpus.
//!
//! For each parity (recto/verso) we take the **median** main-column box —
//! position and size — across every analyzed page. Median rather than mean
//! is deliberate: a single bad scan should not bend the reference for every
//! other page.

use crate::RegisterError;
use crate::page::{AnalyzedPage, BoundingBox, Parity};
use crate::paper::TargetCanvas;

/// Reference layout: where the main text column should land on the canvas,
/// separately for recto and verso pages.
#[derive(Debug, Clone)]
pub struct ReferenceLayout {
    /// Target canvas every page is rendered onto.
    pub canvas: TargetCanvas,
    /// Per-parity reference: target box for the main column on the canvas.
    pub recto: BoundingBox,
    /// Per-parity reference: target box for the main column on the canvas.
    pub verso: BoundingBox,
}

/// Derive a [`ReferenceLayout`] by taking the per-parity median main-column
/// box and centering it on the canvas.
///
/// `pages` is an iterator of borrows so callers can pass `Vec<AnalyzedPage>`,
/// `&[AnalyzedPage]`, or a filtered iterator over a mixed-result corpus
/// without cloning the heavy bitonal buffers.
///
/// # Errors
///
/// Returns [`RegisterError::EmptyCorpus`] if no analyzed pages were supplied.
pub fn derive_reference(
    pages: impl IntoIterator<Item = impl std::borrow::Borrow<AnalyzedPage>>,
    canvas: TargetCanvas,
) -> Result<ReferenceLayout, RegisterError> {
    let snapshots: Vec<(Parity, BoundingBox)> = pages
        .into_iter()
        .map(|p| {
            let page: &AnalyzedPage = p.borrow();
            (page.parity(), page.layout().main_column)
        })
        .collect();
    derive_reference_from_snapshots(&snapshots, canvas)
}

/// Like [`derive_reference`] but takes pre-extracted `(Parity, BoundingBox)`
/// snapshots.
///
/// Saves a clone when the caller already has them around (e.g. a streaming
/// coordinator that accumulates bboxes as workers report in).
///
/// # Errors
///
/// Returns [`RegisterError::EmptyCorpus`] when `snapshots` is empty.
pub fn derive_reference_from_snapshots(
    snapshots: &[(Parity, BoundingBox)],
    canvas: TargetCanvas,
) -> Result<ReferenceLayout, RegisterError> {
    if snapshots.is_empty() {
        return Err(RegisterError::EmptyCorpus);
    }
    let recto = median_for(snapshots, Parity::Recto).unwrap_or_else(|| overall_median(snapshots));
    let verso = median_for(snapshots, Parity::Verso).unwrap_or_else(|| overall_median(snapshots));
    Ok(ReferenceLayout {
        canvas,
        recto: center_on_canvas(recto, canvas),
        verso: center_on_canvas(verso, canvas),
    })
}

impl ReferenceLayout {
    /// Pick the reference box for a given parity.
    #[must_use]
    pub const fn for_parity(&self, parity: Parity) -> BoundingBox {
        match parity {
            Parity::Recto => self.recto,
            Parity::Verso => self.verso,
        }
    }
}

fn median_for(snapshots: &[(Parity, BoundingBox)], parity: Parity) -> Option<BoundingBox> {
    let boxes: Vec<BoundingBox> = snapshots
        .iter()
        .filter(|(p, _)| *p == parity)
        .map(|(_, b)| *b)
        .collect();
    median_of(&boxes)
}

fn overall_median(snapshots: &[(Parity, BoundingBox)]) -> BoundingBox {
    let boxes: Vec<BoundingBox> = snapshots.iter().map(|(_, b)| *b).collect();
    // caller guaranteed non-empty.
    median_of(&boxes).unwrap_or(BoundingBox {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    })
}

fn median_of(boxes: &[BoundingBox]) -> Option<BoundingBox> {
    if boxes.is_empty() {
        return None;
    }
    let widths = median_u32(boxes.iter().map(|b| b.width));
    let heights = median_u32(boxes.iter().map(|b| b.height));
    let xs = median_u32(boxes.iter().map(|b| b.x));
    let ys = median_u32(boxes.iter().map(|b| b.y));
    Some(BoundingBox {
        x: xs,
        y: ys,
        width: widths,
        height: heights,
    })
}

fn median_u32(iter: impl IntoIterator<Item = u32>) -> u32 {
    let mut values: Vec<u32> = iter.into_iter().collect();
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        u32::midpoint(values[mid - 1], values[mid])
    } else {
        values[mid]
    }
}

/// Center `bbox` (whose `width`/`height` define the reference size) on the
/// canvas. The reference's `x`/`y` thus become canvas coordinates, not
/// source-page coordinates.
const fn center_on_canvas(bbox: BoundingBox, canvas: TargetCanvas) -> BoundingBox {
    let x = canvas.width.saturating_sub(bbox.width) / 2;
    let y = canvas.height.saturating_sub(bbox.height) / 2;
    BoundingBox {
        x,
        y,
        width: bbox.width,
        height: bbox.height,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::derive_reference;
    use crate::bitplane::BitPage;
    use crate::page::{BoundingBox, PageLayout, Parity, RawPage, into_analyzed};
    use crate::paper::TargetCanvas;

    fn fake_page(index: usize, x: u32, y: u32, w: u32, h: u32) -> crate::page::AnalyzedPage {
        let raw = RawPage::from_validated(
            BitPage::new_white(100, 100),
            PathBuf::from(format!("p{index}.pbm")),
            index,
        );
        let layout = PageLayout {
            main_column: BoundingBox {
                x,
                y,
                width: w,
                height: h,
            },
            pillars: Vec::new(),
        };
        into_analyzed(raw, layout, Parity::from_index(index))
    }

    #[test]
    fn empty_corpus_errors() {
        let canvas = TargetCanvas {
            width: 100,
            height: 100,
            dpi: 100,
        };
        let empty: [&crate::page::AnalyzedPage; 0] = [];
        assert!(derive_reference(empty, canvas).is_err());
    }

    #[test]
    fn median_box_is_centered_on_canvas() {
        let pages = [
            fake_page(0, 10, 10, 20, 30),
            fake_page(2, 12, 14, 20, 30),
            fake_page(4, 8, 8, 20, 30),
        ];
        let canvas = TargetCanvas {
            width: 100,
            height: 100,
            dpi: 100,
        };
        let r = derive_reference(pages.iter(), canvas).unwrap();
        // recto width=20, height=30 → centered on 100x100 → x=(100-20)/2=40, y=(100-30)/2=35
        assert_eq!(
            r.recto,
            BoundingBox {
                x: 40,
                y: 35,
                width: 20,
                height: 30
            }
        );
    }

    #[test]
    fn verso_falls_back_to_recto_when_absent() {
        let pages = [fake_page(0, 5, 5, 10, 15), fake_page(2, 7, 7, 10, 15)];
        let canvas = TargetCanvas {
            width: 50,
            height: 50,
            dpi: 100,
        };
        let r = derive_reference(pages.iter(), canvas).unwrap();
        // verso has no pages; the fallback uses the overall median (same as recto here).
        assert_eq!(r.recto.width, 10);
        assert_eq!(r.verso.width, 10);
    }
}
