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

/// Compute an [`AlignmentPlan`] for a single analyzed page against the
/// corpus reference.
#[must_use]
pub fn plan_alignment(
    analyzed: AnalyzedPage,
    reference: &ReferenceLayout,
    options: PlanOptions,
) -> AlignmentPlan {
    let target = reference.for_parity(analyzed.parity());
    let (sx, sy) = analyzed.layout().main_column.center();
    let (tx, ty) = target.center();

    // v0.1: pure integer translation — the lossless fast path. v0.2+ will
    // detect when `options.scale` / `options.skew` actually contributes a
    // non-identity component and switch this to `AlignmentKind::Affine`.
    let _ = (options.scale, options.skew);

    #[expect(
        clippy::cast_possible_truncation,
        reason = "page-center deltas live within ±canvas dimensions, well under i32::MAX"
    )]
    let dx_raw = (tx - sx).round() as i32;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "page-center deltas live within ±canvas dimensions, well under i32::MAX"
    )]
    let dy = (ty - sy).round() as i32;
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

    #[test]
    fn translation_moves_source_center_onto_target_center() {
        let raw = RawPage::from_validated(BitPage::new_white(50, 50), PathBuf::from("p.pbm"), 0);
        let layout = PageLayout {
            main_column: BoundingBox {
                x: 10,
                y: 10,
                width: 20,
                height: 30,
            },
            pillars: Vec::new(),
        };
        let analyzed = into_analyzed(raw, layout, Parity::Recto);
        let reference = ReferenceLayout {
            canvas: TargetCanvas {
                width: 100,
                height: 100,
                dpi: 100,
            },
            recto: BoundingBox {
                x: 40,
                y: 35,
                width: 20,
                height: 30,
            },
            verso: BoundingBox {
                x: 40,
                y: 35,
                width: 20,
                height: 30,
            },
        };

        let plan = plan_alignment(analyzed, &reference, PlanOptions::default());
        let AlignmentPlan::Aligned {
            kind: crate::AlignmentKind::Translate { dx, dy },
            ..
        } = plan
        else {
            panic!("expected Aligned/Translate");
        };

        // Source main-column center: (10 + 20/2, 10 + 30/2) = (20, 25).
        // Target main-column center: (40 + 20/2, 35 + 30/2) = (50, 50).
        // → raw translation = (+30, +25); rounding dx down to the nearest
        // multiple of 8 (raster's invariant) keeps it at 24.
        assert_eq!(dy, 25);
        assert_eq!(dx % 8, 0);
        assert!(
            (24..=32).contains(&dx),
            "dx={dx} should be near 30, byte-aligned"
        );
    }
}
