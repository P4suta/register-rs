//! Build the per-page affine transform that maps source coordinates onto the
//! reference layout.
//!
//! Currently translation-only — the v0.1 contract is "no destructive
//! resampling". Scale (v0.2) and skew correction (v0.3) compose into the same
//! [`Projection`](imageproc::geometric_transformations::Projection) so that
//! rendering still applies a single pass and interpolation never stacks.

use crate::page::{AlignmentKind, AlignmentPlan, AnalyzedPage, RawPage};
use crate::reference::ReferenceLayout;

/// Knobs governing which alignment components to compute.
///
/// Disabling components is a destructive-write opt-out, not a tuning knob.
/// The default is "all on" so that callers get the "一発でイイ感じ" output.
#[derive(Debug, Clone, Copy)]
pub struct PlanOptions {
    /// If `false`, skip the per-page scale normalization step.
    pub scale: bool,
    /// If `false`, skip the per-page skew correction step.
    pub skew: bool,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            scale: true,
            skew: true,
        }
    }
}

/// Pages whose main-column bbox is less than this fraction of the corpus
/// reference, in either dimension, are treated as outliers and fall
/// through to passthrough instead of being aligned.
///
/// Chapter openers, frontispieces, and other layouts where the main column
/// occupies only a small fraction of the page are the typical victims of
/// center-based alignment: their bbox center sits well away from the median
/// page's bbox center, so the calculated translation drags the (off-bbox)
/// title or chapter decoration wildly out of place. Refusing to align them
/// keeps their natural layout intact.
const OUTLIER_RATIO_PERCENT: u32 = 50;

/// Compute an [`AlignmentPlan`] for a single analyzed page against the
/// corpus reference.
///
/// Alignment anchors on the **top-right corner** of the main-column bbox,
/// not the center. For Japanese vertical text that's where the first column
/// of body text begins; it's stable across pages that vary in content
/// extent (chapter ends with fewer columns, partial pages, etc.), whereas
/// the bbox center drifts as content density changes.
///
/// Pages whose bbox is significantly smaller than the corpus reference are
/// treated as outliers and fall through to passthrough.
#[must_use]
pub fn plan_alignment(
    analyzed: AnalyzedPage,
    reference: &ReferenceLayout,
    options: PlanOptions,
) -> AlignmentPlan {
    let target = reference.for_parity(analyzed.parity());
    let src = analyzed.layout().main_column;

    // Outlier detection. The main column on a chapter opener or
    // image-heavy page is much smaller than the median; aligning by any
    // anchor on such a small bbox imports its (idiosyncratic) position
    // onto the canvas — including off-bbox content like the chapter
    // title. Skip those pages and let them keep their natural layout.
    let w_ratio_ok = src.width * 100 >= target.width.max(1) * OUTLIER_RATIO_PERCENT;
    let h_ratio_ok = src.height * 100 >= target.height.max(1) * OUTLIER_RATIO_PERCENT;
    if !(w_ratio_ok && h_ratio_ok) {
        return AlignmentPlan::Passthrough { raw: analyzed.raw };
    }

    // Top-right anchor: for Japanese vertical text the rightmost column is
    // the first one read, and the top of that column is where the page
    // "begins". That corner stays put across content-extent variations
    // (end-of-chapter pages where the leftmost columns are missing, pages
    // where the body runs short, etc.), so a translation matching it
    // corrects per-page scan jitter without dragging the layout around.
    let src_anchor_x = src.x.saturating_add(src.width) as i32;
    let src_anchor_y = src.y as i32;
    let target_anchor_x = target.x.saturating_add(target.width) as i32;
    let target_anchor_y = target.y as i32;

    // v0.1: pure integer translation — the lossless fast path. v0.2+ will
    // detect when `options.scale` / `options.skew` actually contributes a
    // non-identity component and switch this to `AlignmentKind::Affine`.
    let _ = (options.scale, options.skew);

    let dx_raw = target_anchor_x - src_anchor_x;
    let dy = target_anchor_y - src_anchor_y;
    // Round dx toward 0 to the nearest multiple of 8 so the `BitPage` blit
    // in [`crate::raster`] stays a pure byte-aligned memcpy. The visual
    // effect is a ≤ 7-pixel horizontal nudge — invisible at 300 DPI on
    // bitonal novel text.
    let dx = round_to_byte(dx_raw);
    AlignmentPlan::Aligned {
        analyzed,
        kind: AlignmentKind::Translate { dx, dy },
    }
}

#[inline]
const fn round_to_byte(x: i32) -> i32 {
    // Round half-to-zero for cleanest visual symmetry across recto/verso.
    let bias = if x >= 0 { 4 } else { -4 };
    ((x + bias) / 8) * 8
}

/// Wrap a raw page that bypassed analysis as a [`Passthrough`](AlignmentPlan::Passthrough)
/// plan. Centered placement on the canvas happens in [`crate::raster::render`].
#[must_use]
pub const fn plan_passthrough(raw: RawPage) -> AlignmentPlan {
    AlignmentPlan::Passthrough { raw }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{PlanOptions, plan_alignment};
    use crate::bitplane::BitPage;
    use crate::page::{AlignmentPlan, BoundingBox, PageLayout, Parity, RawPage, into_analyzed};
    use crate::paper::TargetCanvas;
    use crate::reference::ReferenceLayout;

    fn make_ref(w: u32, h: u32, bbox: BoundingBox) -> ReferenceLayout {
        ReferenceLayout {
            canvas: TargetCanvas {
                width: w,
                height: h,
                dpi: 100,
            },
            recto: bbox,
            verso: bbox,
        }
    }

    fn analyzed_page(main_column: BoundingBox) -> crate::page::AnalyzedPage {
        let raw = RawPage::from_validated(BitPage::new_white(50, 50), PathBuf::from("p.pbm"), 0);
        let layout = PageLayout {
            main_column,
            pillars: Vec::new(),
        };
        into_analyzed(raw, layout, Parity::Recto)
    }

    #[test]
    fn translation_anchors_on_top_right_corner() {
        // Source top-right = (30, 10); target top-right = (60, 35).
        // → raw translation = (+30, +25); dx byte-aligned (24 or 32).
        let analyzed = analyzed_page(BoundingBox {
            x: 10,
            y: 10,
            width: 20,
            height: 30,
        });
        let reference = make_ref(
            100,
            100,
            BoundingBox {
                x: 40,
                y: 35,
                width: 20,
                height: 30,
            },
        );

        let plan = plan_alignment(analyzed, &reference, PlanOptions::default());
        let AlignmentPlan::Aligned {
            kind: crate::AlignmentKind::Translate { dx, dy },
            ..
        } = plan
        else {
            panic!("expected Aligned/Translate");
        };

        assert_eq!(dy, 25);
        assert_eq!(dx % 8, 0);
        assert!(
            (24..=32).contains(&dx),
            "dx={dx} should be near 30, byte-aligned"
        );
    }

    #[test]
    fn small_outlier_bbox_falls_through_to_passthrough() {
        // A "chapter opener" — bbox is < 50% of reference width.
        // Should not be aligned; the page keeps its natural layout.
        let analyzed = analyzed_page(BoundingBox {
            x: 80,
            y: 100,
            width: 20, // 20% of target width=100
            height: 100,
        });
        let reference = make_ref(
            200,
            200,
            BoundingBox {
                x: 5,
                y: 5,
                width: 100,
                height: 100,
            },
        );
        let plan = plan_alignment(analyzed, &reference, PlanOptions::default());
        assert!(
            matches!(plan, AlignmentPlan::Passthrough { .. }),
            "outlier bbox should passthrough; got {plan:?}",
        );
    }

    #[test]
    fn end_of_chapter_short_page_still_aligns_when_width_matches() {
        // End-of-chapter page: bbox width matches reference, but height
        // is ~70% of reference. With top-right anchor + 50% outlier
        // threshold, it should still align (and the alignment should be
        // a no-op since the top-right anchor is at the same position).
        let analyzed = analyzed_page(BoundingBox {
            x: 5,
            y: 5,
            width: 100,
            height: 70,
        });
        let reference = make_ref(
            200,
            200,
            BoundingBox {
                x: 5,
                y: 5,
                width: 100,
                height: 100,
            },
        );
        let plan = plan_alignment(analyzed, &reference, PlanOptions::default());
        let AlignmentPlan::Aligned {
            kind: crate::AlignmentKind::Translate { dx, dy },
            ..
        } = plan
        else {
            panic!("expected Aligned/Translate");
        };
        // Top-right corner of both is (105, 5), so dx, dy = 0.
        assert_eq!((dx, dy), (0, 0));
    }
}
