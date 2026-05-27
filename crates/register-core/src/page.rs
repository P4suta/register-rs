//! Page types — one per pipeline stage.
//!
//! Each stage is its own type, so the only way to produce a [`RegisteredPage`]
//! is to walk the full pipeline and the only way to feed [`raster::render`]
//! is with an [`AlignmentPlan`]. Illegal compositions are compile errors.

use std::path::{Path, PathBuf};

use crate::bitplane::BitPage;

/// A page that has just been loaded from disk. Position, paper size, and
/// even rotation are still unknown to the rest of the pipeline.
///
/// The bits are stored in their native 1-bit packing — see [`BitPage`] —
/// so a no-op pipeline does not unpack and re-pack on every page.
#[derive(Debug, Clone)]
pub struct RawPage {
    pub(crate) bits: BitPage,
    pub(crate) source: PathBuf,
    pub(crate) index: usize,
}

impl RawPage {
    /// Construct a [`RawPage`] from an already-validated bitonal buffer.
    ///
    /// `index` is the page's position in the corpus walk order, used to
    /// derive parity (recto/verso) in [`crate::analyze`].
    #[must_use]
    pub(crate) const fn from_validated(bits: BitPage, source: PathBuf, index: usize) -> Self {
        Self {
            bits,
            source,
            index,
        }
    }

    /// Source path the page was loaded from.
    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// Corpus-order index, starting at 0.
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Pixel dimensions of the loaded page.
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.bits.width(), self.bits.height())
    }

    /// Borrow the underlying bit-packed buffer.
    #[must_use]
    pub const fn bits(&self) -> &BitPage {
        &self.bits
    }
}

/// A page whose main text column has been located and whose parity has been
/// determined. Still pure-1-bit — no resampling has happened.
#[derive(Debug, Clone)]
pub struct AnalyzedPage {
    pub(crate) raw: RawPage,
    pub(crate) layout: PageLayout,
    pub(crate) parity: Parity,
}

impl AnalyzedPage {
    /// Borrow the raw page this analysis was derived from.
    #[must_use]
    pub const fn raw(&self) -> &RawPage {
        &self.raw
    }

    /// Detected layout (main column + ancillary regions).
    #[must_use]
    pub const fn layout(&self) -> &PageLayout {
        &self.layout
    }

    /// Page parity: which side of the spread this page sits on.
    #[must_use]
    pub const fn parity(&self) -> Parity {
        self.parity
    }
}

/// Plan for moving one page onto the reference canvas.
///
/// Two variants make the failure mode explicit at the type level: pages whose
/// main column could not be detected fall through to [`AlignmentPlan::Passthrough`]
/// and land on the canvas centered, without any destructive resampling.
#[derive(Debug, Clone)]
pub enum AlignmentPlan {
    /// The page's main column was detected. The [`AlignmentKind`] decides
    /// whether [`crate::raster::render`] can take the lossless fast path.
    Aligned {
        /// The analyzed page being aligned.
        analyzed: AnalyzedPage,
        /// What geometric transform takes the source to the canvas.
        kind: AlignmentKind,
    },
    /// Column detection failed (blank page, frontispiece, etc.). The page is
    /// centered on the canvas as-is.
    Passthrough {
        /// The raw page that could not be analyzed.
        raw: RawPage,
    },
}

impl AlignmentPlan {
    /// The source path of the page this plan applies to.
    #[must_use]
    pub fn source(&self) -> &Path {
        match self {
            Self::Aligned { analyzed, .. } => analyzed.raw.source(),
            Self::Passthrough { raw } => raw.source(),
        }
    }
}

/// Specialization of the per-page transform.
///
/// A separate `Translate` variant lets [`crate::raster::render`] dispatch
/// pure integer translation through a memcpy fast path — bit-exact, lossless,
/// and ~5× faster than running the general affine warp. Once v0.2 introduces
/// scale and v0.3 introduces skew, those flow through [`AlignmentKind::Affine`]
/// and pay the resampling cost only when they actually need to.
#[derive(Debug, Clone, Copy)]
pub enum AlignmentKind {
    /// Pure integer translation by `(dx, dy)` pixels. Lossless.
    Translate {
        /// X translation in pixels (positive = right).
        dx: i32,
        /// Y translation in pixels (positive = down).
        dy: i32,
    },
    /// General affine — needed for scale, skew, or sub-pixel translation.
    /// Goes through bilinear warp + re-binarisation.
    Affine(imageproc::geometric_transformations::Projection),
}

/// A page rendered onto the fixed paper-size canvas and re-binarised.
///
/// Pixel dimensions are guaranteed equal to the target canvas dimensions,
/// so every output of a single corpus run is the same size.
#[derive(Debug, Clone)]
pub struct RegisteredPage {
    pub(crate) bits: BitPage,
    pub(crate) source: PathBuf,
}

impl RegisteredPage {
    /// Source path the page came from (carried through for output mirroring).
    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// The final bit-packed buffer to write out.
    #[must_use]
    pub const fn bits(&self) -> &BitPage {
        &self.bits
    }
}

/// Which side of a two-page spread a page lies on.
///
/// For vertically-typeset Japanese novels the gutter (ノド) sits on the **right**
/// of a recto page and on the **left** of a verso page, so the main text column
/// is mirrored across the spread. The pipeline keeps separate reference layouts
/// per parity to avoid introducing a new even/odd jitter while trying to remove
/// the per-scan jitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    /// Right-hand page in a Japanese (right-to-left) book — typically the
    /// page whose corpus index is **even** (0-based).
    Recto,
    /// Left-hand page in a Japanese book — typically the page whose corpus
    /// index is **odd**.
    Verso,
}

impl Parity {
    /// Derive parity from a zero-based corpus index, assuming the first page
    /// in sort order is the leading recto.
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        if index.is_multiple_of(2) {
            Self::Recto
        } else {
            Self::Verso
        }
    }
}

/// What [`analyze`](crate::analyze::analyze) extracts from a page.
#[derive(Debug, Clone)]
pub struct PageLayout {
    /// The bounding box of the dense text region (the main column block).
    pub main_column: BoundingBox,
    /// Ancillary marks outside the main column — ノンブル, running titles, etc.
    /// Carried through so future versions can preserve their relative position
    /// without re-using them as anchors.
    pub pillars: Vec<BoundingBox>,
}

/// An axis-aligned pixel-space bounding box. `x`/`y` are inclusive top-left
/// coordinates; `width`/`height` are positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundingBox {
    /// Left edge in pixels.
    pub x: u32,
    /// Top edge in pixels.
    pub y: u32,
    /// Box width in pixels.
    pub width: u32,
    /// Box height in pixels.
    pub height: u32,
}

impl BoundingBox {
    /// Geometric center of the box, in subpixel coordinates.
    #[must_use]
    pub fn center(self) -> (f32, f32) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "page dimensions easily fit in f32 mantissa"
        )]
        let cx = self.x as f32 + self.width as f32 / 2.0;
        #[expect(
            clippy::cast_precision_loss,
            reason = "page dimensions easily fit in f32 mantissa"
        )]
        let cy = self.y as f32 + self.height as f32 / 2.0;
        (cx, cy)
    }
}

/// Internal helper: lift a raw page + detected layout + parity into an
/// `AnalyzedPage`. Kept crate-private so the only path through the pipeline
/// is via [`crate::analyze::analyze`].
pub(crate) const fn into_analyzed(
    raw: RawPage,
    layout: PageLayout,
    parity: Parity,
) -> AnalyzedPage {
    AnalyzedPage {
        raw,
        layout,
        parity,
    }
}
