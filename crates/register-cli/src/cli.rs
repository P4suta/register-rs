//! Command-line argument parsing.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Align bitonal Japanese-novel scan pages onto a fixed paper-size canvas.
///
/// Walks `<INPUT_DIR>` in sort order (= page order), analyses each page's
/// main text column, derives a per-parity reference layout from the corpus,
/// and writes one registered page per input into `<OUTPUT_DIR>`.
#[derive(Debug, Parser)]
#[command(name = "register", version, about, long_about = None)]
pub(crate) struct Args {
    /// Directory containing bitonal page images (read recursively).
    pub(crate) input_dir: PathBuf,

    /// Directory to write registered images into.
    pub(crate) output_dir: PathBuf,

    /// Paper standard for the output canvas. See `register --help` for
    /// the pixel dimensions each value produces at common DPIs. Default
    /// `shiroku` (127 × 188 mm) matches the typical Japanese hardcover
    /// novel trim — change it if you're scanning a different trim size.
    #[arg(long, value_enum, default_value_t = PaperArg::Shiroku)]
    pub(crate) paper: PaperArg,

    /// Output DPI used to convert paper millimeters to pixels.
    #[arg(long, default_value_t = 400)]
    pub(crate) dpi: u32,

    /// Output image format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Same)]
    pub(crate) format: OutputFormat,

    /// Glob pattern selecting input file names.
    #[arg(long, default_value = "*.{pbm,png,tiff,tif}")]
    pub(crate) glob: String,

    /// Number of worker threads. Defaults to the logical CPU count.
    #[arg(short = 'j', long)]
    pub(crate) jobs: Option<usize>,

    /// Overwrite output directory contents if non-empty.
    #[arg(long)]
    pub(crate) force: bool,

    /// Skip per-page scale normalization (translation-only output).
    #[arg(long)]
    pub(crate) no_scale: bool,

    /// Skip per-page skew correction (translation/scale-only output).
    #[arg(long)]
    pub(crate) no_skew: bool,
}

/// Paper-standard selection. Names mirror the JIS / ISO standards and the
/// Japanese book-trim vocabulary (新書 = shinsho, 四六 = shiroku); see
/// `register_core::Paper` for exact mm dimensions and pre-rounded pixel
/// sizes at common DPIs.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum PaperArg {
    /// ISO A4 — 210 × 297 mm. Manga / magazines.
    A4,
    /// ISO A5 — 148 × 210 mm. Large novels, academic books.
    A5,
    /// ISO A6 — 105 × 148 mm. 文庫判 (Japanese paperback).
    A6,
    /// JIS B5 — 182 × 257 mm. Oversize Japanese books, textbooks.
    B5,
    /// JIS B6 — 128 × 182 mm. Compact hardcovers.
    B6,
    /// 新書判 — 103 × 182 mm. Japanese 新書 series.
    Shinsho,
    /// 四六判 — 127 × 188 mm. Mainstream Japanese hardcover novels.
    Shiroku,
    /// ISO B5 — 176 × 250 mm. Western B5; rare on Japanese books.
    IsoB5,
}

impl From<PaperArg> for register_core::Paper {
    fn from(value: PaperArg) -> Self {
        match value {
            PaperArg::A4 => Self::A4,
            PaperArg::A5 => Self::A5,
            PaperArg::A6 => Self::A6,
            PaperArg::B5 => Self::B5,
            PaperArg::B6 => Self::B6,
            PaperArg::Shinsho => Self::Shinsho,
            PaperArg::Shiroku => Self::Shiroku,
            PaperArg::IsoB5 => Self::IsoB5,
        }
    }
}

/// Output image format selection.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum OutputFormat {
    /// Keep the input file's extension and format.
    Same,
    /// Write every output as PBM (P4 binary).
    Pbm,
    /// Write every output as PNG.
    Png,
}
