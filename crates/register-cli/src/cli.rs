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

    /// Paper standard for the output canvas.
    #[arg(long, value_enum, default_value_t = PaperArg::B5)]
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

/// Paper-standard selection.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum PaperArg {
    /// ISO B5 — 182 × 257 mm.
    B5,
}

impl From<PaperArg> for register_core::Paper {
    fn from(value: PaperArg) -> Self {
        match value {
            PaperArg::B5 => Self::B5,
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
