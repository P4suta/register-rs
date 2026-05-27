//! Paper-size canvas: a fixed pixel target every page is registered onto.
//!
//! The whole point of this tool is "every page has *exactly* the same pixel
//! dimensions", so the canvas is established once per run from a [`Paper`]
//! standard plus a DPI, and every [`RegisteredPage`](crate::RegisteredPage)
//! is rasterised into that shape.

/// ISO / JIS paper standards supported as a registration target.
///
/// Members are added on demand. Only B5 — by far the dominant 文庫/単行本
/// trim size for self-scanned novels — is supported initially.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Paper {
    /// ISO B5 — 182 × 257 mm.
    B5,
}

impl Paper {
    /// Physical dimensions in millimeters, returned as `(width, height)` in
    /// portrait orientation.
    #[must_use]
    pub const fn dimensions_mm(self) -> (f64, f64) {
        match self {
            Self::B5 => (182.0, 257.0),
        }
    }

    /// Build a pixel-space canvas description at the given DPI.
    #[must_use]
    pub fn canvas_at_dpi(self, dpi: u32) -> TargetCanvas {
        let (w_mm, h_mm) = self.dimensions_mm();
        TargetCanvas::from_mm(w_mm, h_mm, dpi)
    }
}

/// The pixel-space canvas every page is registered onto, plus the DPI it was
/// derived from. Constructed via [`Paper::canvas_at_dpi`].
#[derive(Debug, Clone, Copy)]
pub struct TargetCanvas {
    /// Canvas width in pixels.
    pub width: u32,
    /// Canvas height in pixels.
    pub height: u32,
    /// Dots-per-inch used to convert physical mm to pixels.
    pub dpi: u32,
}

impl TargetCanvas {
    pub(crate) fn from_mm(width_mm: f64, height_mm: f64, dpi: u32) -> Self {
        const MM_PER_INCH: f64 = 25.4;
        let scale = f64::from(dpi) / MM_PER_INCH;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "paper-size px values are well below u32::MAX at any realistic DPI"
        )]
        let to_px = |mm: f64| (mm * scale).round() as u32;
        Self {
            width: to_px(width_mm),
            height: to_px(height_mm),
            dpi,
        }
    }

    /// Subpixel center of the canvas.
    #[must_use]
    pub fn center(self) -> (f32, f32) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "canvas dims easily fit in f32 mantissa"
        )]
        let cx = self.width as f32 / 2.0;
        #[expect(
            clippy::cast_precision_loss,
            reason = "canvas dims easily fit in f32 mantissa"
        )]
        let cy = self.height as f32 / 2.0;
        (cx, cy)
    }
}

#[cfg(test)]
mod tests {
    use super::{Paper, TargetCanvas};

    #[test]
    fn b5_at_400_dpi_is_2866_by_4047() {
        // 182 mm * 400 / 25.4 = 2866.14… → 2866
        // 257 mm * 400 / 25.4 = 4047.24… → 4047
        let canvas = Paper::B5.canvas_at_dpi(400);
        assert_eq!(canvas.width, 2866);
        assert_eq!(canvas.height, 4047);
        assert_eq!(canvas.dpi, 400);
    }

    #[test]
    fn b5_at_300_dpi_rounds_to_nearest() {
        let canvas = Paper::B5.canvas_at_dpi(300);
        // 182 * 300 / 25.4 = 2149.6… → 2150; 257 * 300 / 25.4 = 3035.4… → 3035
        assert_eq!(canvas.width, 2150);
        assert_eq!(canvas.height, 3035);
    }

    #[test]
    fn center_is_half_of_dimensions() {
        let canvas = TargetCanvas {
            width: 200,
            height: 300,
            dpi: 100,
        };
        assert!((canvas.center().0 - 100.0).abs() < f32::EPSILON);
        assert!((canvas.center().1 - 150.0).abs() < f32::EPSILON);
    }
}
