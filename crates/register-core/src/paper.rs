//! Paper-size canvas — a fixed pixel target every page is registered onto.
//!
//! The whole point of this tool is "every page has *exactly* the same pixel
//! dimensions", so the canvas is established once per run from a [`Paper`]
//! standard plus a DPI, and every [`RegisteredPage`](crate::RegisteredPage)
//! is rasterised into that shape.
//!
//! ## How a paper standard becomes pixels
//!
//! Each [`Paper`] variant is defined by its physical width and height in
//! millimeters (the dimensions printed on stationery packaging). At runtime
//! the user supplies a DPI; the canvas size in pixels is:
//!
//! ```text
//! width_px  = round(width_mm  * dpi / 25.4)
//! height_px = round(height_mm * dpi / 25.4)
//! ```
//!
//! `25.4` is the exact mm-to-inch conversion. The round-to-nearest at the
//! end is the only floating-point fudge: mm-to-px for arbitrary paper
//! sizes and DPIs never divides cleanly, so the closest integer is the
//! best we can do without redefining what a "millimeter" or a "DPI" is.
//!
//! Pre-rounded pixel sizes for the standards below are listed in their
//! doc-comments at the three most common DPIs so you can pick a DPI that
//! gives you the size you want without doing the arithmetic.
//!
//! ## Which standard to pick for a Japanese book?
//!
//! Approximate guide (compare against `pdftoppm -mono -r 300 *.pdf` output
//! dimensions ÷ 11.811 to get mm):
//!
//! | Source page (mm)   | Standard | Common Japanese name |
//! |--------------------|----------|----------------------|
//! | ~105 × 148         | [`Paper::A6`]     | 文庫判 (paperback)   |
//! | ~103 × 182         | [`Paper::Shinsho`] | 新書判              |
//! | ~127 × 188         | [`Paper::Shiroku`] | 四六判 (hardcover)  |
//! | ~128 × 182         | [`Paper::B6`]     | B6 (compact hardcover) |
//! | ~148 × 210         | [`Paper::A5`]     | A5 (large novel)    |
//! | ~182 × 257         | [`Paper::B5`]     | B5 (oversize)       |
//! | ~210 × 297         | [`Paper::A4`]     | A4 (manga, magazines) |

/// Supported paper standards.
///
/// All dimensions are exact in millimeters per JIS / ISO definitions; the
/// pixel canvas is derived from them at run-time (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Paper {
    /// ISO A4 — 210 × 297 mm. Manga, magazines, scientific paper.
    ///
    /// Pre-rounded canvas sizes: 200 DPI → 1654 × 2339; 300 DPI →
    /// 2480 × 3508; 400 DPI → 3307 × 4677; 600 DPI → 4961 × 7016.
    A4,
    /// ISO A5 — 148 × 210 mm. Large novels and academic books.
    ///
    /// Pre-rounded canvas sizes: 200 DPI → 1165 × 1654; 300 DPI →
    /// 1748 × 2480; 400 DPI → 2331 × 3307; 600 DPI → 3496 × 4961.
    A5,
    /// ISO A6 — 105 × 148 mm. 文庫判 — the Japanese paperback standard.
    ///
    /// Pre-rounded canvas sizes: 200 DPI → 827 × 1165; 300 DPI →
    /// 1240 × 1748; 400 DPI → 1654 × 2331; 600 DPI → 2480 × 3496.
    A6,
    /// JIS B5 — 182 × 257 mm. Some Japanese novels (notably 単行本 large
    /// format) and many textbooks. Different from ISO B5 (176 × 250 mm).
    ///
    /// Pre-rounded canvas sizes: 200 DPI → 1433 × 2024; 300 DPI →
    /// 2150 × 3035; 400 DPI → 2866 × 4047; 600 DPI → 4299 × 6071.
    B5,
    /// JIS B6 — 128 × 182 mm. Compact hardcovers (e.g. 河出文庫, 角川文庫
    /// in their larger editions).
    ///
    /// Pre-rounded canvas sizes: 200 DPI → 1008 × 1433; 300 DPI →
    /// 1512 × 2150; 400 DPI → 2016 × 2866; 600 DPI → 3024 × 4299.
    B6,
    /// 新書判 — 103 × 182 mm. The 岩波新書 / 講談社現代新書 / 中公新書
    /// trim size; widely used for non-fiction series.
    ///
    /// Pre-rounded canvas sizes: 200 DPI → 811 × 1433; 300 DPI →
    /// 1217 × 2150; 400 DPI → 1622 × 2866; 600 DPI → 2433 × 4299.
    Shinsho,
    /// 四六判 — 127 × 188 mm. The dominant trim for Japanese mainstream
    /// hardcover and large-format paperback novels.
    ///
    /// Pre-rounded canvas sizes: 200 DPI → 1000 × 1480; 300 DPI →
    /// 1500 × 2220; 400 DPI → 2000 × 2961; 600 DPI → 3000 × 4441.
    Shiroku,
    /// ISO B5 — 176 × 250 mm. Western B5; rare on Japanese books, included
    /// for completeness when scanning imported titles.
    ///
    /// Pre-rounded canvas sizes: 200 DPI → 1386 × 1969; 300 DPI →
    /// 2079 × 2953; 400 DPI → 2772 × 3937; 600 DPI → 4157 × 5906.
    IsoB5,
}

impl Paper {
    /// Physical dimensions in millimeters, in portrait orientation
    /// (`width <= height`).
    #[must_use]
    pub const fn dimensions_mm(self) -> (f64, f64) {
        match self {
            Self::A4 => (210.0, 297.0),
            Self::A5 => (148.0, 210.0),
            Self::A6 => (105.0, 148.0),
            Self::B5 => (182.0, 257.0),
            Self::B6 => (128.0, 182.0),
            Self::Shinsho => (103.0, 182.0),
            Self::Shiroku => (127.0, 188.0),
            Self::IsoB5 => (176.0, 250.0),
        }
    }

    /// Convert this paper standard to a pixel-space canvas at the given DPI.
    ///
    /// The pixel dimensions are `round(width_mm * dpi / 25.4)`; see the
    /// module docs for why the round-to-nearest is the best you can do
    /// without redefining the relevant ISO / JIS standards.
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
        let cx = self.width as f32 / 2.0;
        let cy = self.height as f32 / 2.0;
        (cx, cy)
    }
}

#[cfg(test)]
mod tests {
    use super::Paper;

    /// Every standard's pre-rounded sizes match the doc-comment table at
    /// the three most common DPIs (200 / 300 / 400 / 600). If the doc
    /// numbers ever drift from the code, this test fails so the user
    /// doesn't get bitten by the inconsistency.
    #[test]
    fn pre_rounded_canvas_sizes_match_docs() {
        for &(paper, dpi, expected) in &[
            (Paper::A4, 200, (1654_u32, 2339_u32)),
            (Paper::A4, 300, (2480, 3508)),
            (Paper::A4, 400, (3307, 4677)),
            (Paper::A4, 600, (4961, 7016)),
            (Paper::A5, 200, (1165, 1654)),
            (Paper::A5, 300, (1748, 2480)),
            (Paper::A5, 400, (2331, 3307)),
            (Paper::A5, 600, (3496, 4961)),
            (Paper::A6, 200, (827, 1165)),
            (Paper::A6, 300, (1240, 1748)),
            (Paper::A6, 400, (1654, 2331)),
            (Paper::A6, 600, (2480, 3496)),
            (Paper::B5, 200, (1433, 2024)),
            (Paper::B5, 300, (2150, 3035)),
            (Paper::B5, 400, (2866, 4047)),
            (Paper::B5, 600, (4299, 6071)),
            (Paper::B6, 200, (1008, 1433)),
            (Paper::B6, 300, (1512, 2150)),
            (Paper::B6, 400, (2016, 2866)),
            (Paper::B6, 600, (3024, 4299)),
            (Paper::Shinsho, 200, (811, 1433)),
            (Paper::Shinsho, 300, (1217, 2150)),
            (Paper::Shinsho, 400, (1622, 2866)),
            (Paper::Shinsho, 600, (2433, 4299)),
            (Paper::Shiroku, 200, (1000, 1480)),
            (Paper::Shiroku, 300, (1500, 2220)),
            (Paper::Shiroku, 400, (2000, 2961)),
            (Paper::Shiroku, 600, (3000, 4441)),
            (Paper::IsoB5, 200, (1386, 1969)),
            (Paper::IsoB5, 300, (2079, 2953)),
            (Paper::IsoB5, 400, (2772, 3937)),
            (Paper::IsoB5, 600, (4157, 5906)),
        ] {
            let canvas = paper.canvas_at_dpi(dpi);
            assert_eq!(
                (canvas.width, canvas.height),
                expected,
                "{paper:?} @ {dpi} DPI: got {}x{}, doc-comment says {:?}",
                canvas.width,
                canvas.height,
                expected,
            );
        }
    }

    #[test]
    fn center_is_half_of_dimensions() {
        let canvas = Paper::B5.canvas_at_dpi(300);
        let (cx, cy) = canvas.center();
        assert!((cx - 1075.0).abs() < f32::EPSILON);
        assert!((cy - 1517.5).abs() < f32::EPSILON);
    }
}
