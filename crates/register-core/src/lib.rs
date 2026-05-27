//! Core registration primitives for **register-rs**.
//!
//! Given a corpus of bitonal scanned novel pages whose paper edges, text-block
//! positions, and slight rotation/scale all vary, this crate computes a
//! per-page affine transform that lands every page's main text column on a
//! shared reference layout inside a fixed paper-size canvas (e.g. ISO B5).
//!
//! # Pipeline
//!
//! The work is staged through a chain of types that make illegal compositions
//! a compile error:
//!
//! ```text
//!   RawPage  ─analyze─▶ AnalyzedPage  ─plan─▶ AlignmentPlan  ─render─▶ RegisteredPage
//! ```
//!
//! - [`analyze::analyze`] locates the main text column and the page's parity
//!   (recto/verso) — a pure 1-bit operation.
//! - [`reference::derive_reference`] aggregates the per-parity median layout
//!   over the whole corpus, so every page is positioned relative to the same
//!   normative target — also pure 1-bit.
//! - [`plan::plan_alignment`] composes translation, rotation, and uniform
//!   scale into a single [`imageproc::geometric_transformations::Projection`].
//! - [`raster::render`] is the **sole** destructive site: it lifts to
//!   grayscale, applies the affine in a single pass to avoid interpolation
//!   stacking, re-binarises via Otsu, and pastes the result onto the
//!   fixed-size paper canvas.
//!
//! Directory traversal, parallelism, progress reporting, and logging all
//! live in `register-cli` so that this crate stays a pure library — reusable
//! from tests, examples, and future GUI / WASM frontends without dragging
//! that machinery along.

#![deny(unsafe_code)]

mod analyze;
mod bitplane;
mod error;
mod io;
mod page;
mod paper;
mod plan;
mod raster;
mod reference;

pub use crate::analyze::analyze;
pub use crate::bitplane::BitPage;
pub use crate::error::RegisterError;
pub use crate::io::{load_bitonal, save_bitonal};
pub use crate::page::{
    AlignmentKind, AlignmentPlan, AnalyzedPage, BoundingBox, PageLayout, Parity, RawPage,
    RegisteredPage,
};
pub use crate::paper::{Paper, TargetCanvas};
pub use crate::plan::{PlanOptions, plan_alignment, plan_passthrough};
pub use crate::raster::{CanvasBuf, render, render_into};
pub use crate::reference::{ReferenceLayout, derive_reference, derive_reference_from_snapshots};

/// Test / benchmark helpers. Bypass invariant checks; production code should
/// use the regular constructors via [`load_bitonal`].
#[doc(hidden)]
pub mod test_only {
    use std::path::PathBuf;

    use crate::{BitPage, RawPage};

    /// Build a [`RawPage`] from a [`BitPage`] without going through file I/O.
    #[must_use]
    pub fn raw_from_bits(bits: BitPage, source: PathBuf, index: usize) -> RawPage {
        RawPage::from_validated(bits, source, index)
    }
}
