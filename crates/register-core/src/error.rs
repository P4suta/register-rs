//! Error type produced by the `register-core` API.

use std::io;
use std::path::PathBuf;

/// Errors that can occur while loading, analyzing, or rendering a page.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegisterError {
    /// I/O failure while reading or writing a page image.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being read or written when the error occurred.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// Error returned by the `image` crate while decoding or encoding.
    #[error("image error at {path}: {source}")]
    Image {
        /// The path being read or written when the error occurred.
        path: PathBuf,
        /// The underlying error from the `image` crate.
        #[source]
        source: image::ImageError,
    },

    /// The input page is not a 1-bit bitonal image. `register-core` refuses
    /// to silently binarise — the caller is expected to feed already-binary
    /// scans (e.g. CCITT G4 extracted by `pdfimages`, then optionally
    /// pre-processed by `despeckle`).
    #[error("input image is not bitonal: {path}")]
    NotBitonal {
        /// The path that contains the non-bitonal image.
        path: PathBuf,
    },

    /// The corpus produced no analyzable pages, so a reference layout cannot
    /// be derived. The caller is expected to feed a non-empty corpus.
    #[error("no analyzable pages in corpus (every page failed column detection)")]
    EmptyCorpus,
}
