//! Image I/O for `register-core`.
//!
//! Mirrors `despeckle-core`'s contract: 1-bit PBM/PNG/TIFF in, 1-bit PBM
//! out. The PBM round-trip is **zero-transform**: a [`BitPage`]'s in-memory
//! layout is identical to the on-disk P4 pixel section, so loading is one
//! `fs::read` + a header parse, and saving is a header `write` + one
//! `write` of the bit buffer. The 8× memory bloat and bit-unpack /
//! bit-pack passes that earlier versions of this crate paid on every page
//! are gone.
//!
//! PNG / TIFF inputs still defer to the `image` crate and convert to
//! [`BitPage`] in-process; this path is taken rarely (most corpora are
//! `pdftoppm -mono` PBMs).

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use image::{GrayImage, ImageReader};

use crate::RegisterError;
use crate::bitplane::BitPage;
use crate::page::RawPage;

/// Load a bitonal page image from `path`, refusing to silently binarise.
///
/// `index` is the page's position in the corpus walk order; it is stored in
/// the returned [`RawPage`] and used downstream to derive parity.
///
/// # Errors
///
/// - [`RegisterError::Io`] if `path` cannot be opened.
/// - [`RegisterError::Image`] if the bytes at `path` cannot be decoded.
/// - [`RegisterError::NotBitonal`] if a non-PBM source contains pixels
///   other than `0` or `255` after decoding.
pub fn load_bitonal(path: &Path, index: usize) -> Result<RawPage, RegisterError> {
    let path_buf: PathBuf = path.into();
    if let Some(bp) = load_pbm_p4_if_applicable(&path_buf)? {
        return Ok(RawPage::from_validated(bp, path_buf, index));
    }
    // Slow path: PNG / TIFF / other. Decode via `image`, validate bitonal,
    // then pack into a `BitPage`.
    let reader = ImageReader::open(path).map_err(|source| RegisterError::Io {
        path: path_buf.clone(),
        source,
    })?;
    let dynamic = reader.decode().map_err(|source| RegisterError::Image {
        path: path_buf.clone(),
        source,
    })?;
    let gray = dynamic.to_luma8();
    if !is_effectively_bitonal(&gray) {
        return Err(RegisterError::NotBitonal { path: path_buf });
    }
    let bp = bit_pack_grayscale(&gray);
    Ok(RawPage::from_validated(bp, path_buf, index))
}

/// Save a bitonal page image to `path`.
///
/// `.pbm` paths are written as P4 (binary bitmap, 1 bit per pixel). Other
/// extensions detour through `image::GrayImage` (BitPage → GrayImage
/// upconvert) since PNG / TIFF encoders expect byte-per-pixel input.
///
/// # Errors
///
/// Returns [`RegisterError::Io`] or [`RegisterError::Image`] if writing fails.
pub fn save_bitonal(bits: &BitPage, path: &Path) -> Result<(), RegisterError> {
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("pbm") | None) {
        write_pbm_p4(bits, path)
    } else {
        let gray = crate::raster::bit_unpack_to_grayscale(bits);
        gray.save(path).map_err(|source| RegisterError::Image {
            path: path.into(),
            source,
        })
    }
}

fn write_pbm_p4(bits: &BitPage, path: &Path) -> Result<(), RegisterError> {
    let path_buf: PathBuf = path.into();
    let width = bits.width();
    let height = bits.height();
    let header = format!("P4\n{width} {height}\n");
    let body = bits.bytes();

    let mut payload = Vec::with_capacity(header.len() + body.len());
    payload.extend_from_slice(header.as_bytes());
    payload.extend_from_slice(body);

    let mut file = fs::File::create(path).map_err(|source| RegisterError::Io {
        path: path_buf.clone(),
        source,
    })?;
    file.write_all(&payload)
        .map_err(|source| RegisterError::Io {
            path: path_buf,
            source,
        })
}

/// Returns `Ok(Some(bp))` if the file is a P4 PBM and we decoded it;
/// `Ok(None)` if it's some other format (caller will fall through).
fn load_pbm_p4_if_applicable(path: &Path) -> Result<Option<BitPage>, RegisterError> {
    let bytes = fs::read(path).map_err(|source| RegisterError::Io {
        path: path.into(),
        source,
    })?;
    if bytes.len() < 2 || &bytes[..2] != b"P4" {
        return Ok(None);
    }
    decode_p4(bytes)
        .map(Some)
        .ok_or_else(|| RegisterError::Image {
            path: path.into(),
            source: image::ImageError::Decoding(image::error::DecodingError::new(
                image::error::ImageFormatHint::Name("P4 (PBM)".into()),
                "malformed P4 header",
            )),
        })
}

/// Decode a P4 PBM byte stream into a `BitPage`. Parsing the header costs
/// a handful of bytes; the pixel section is taken as-is — there is no
/// per-pixel transform pass.
fn decode_p4(mut bytes: Vec<u8>) -> Option<BitPage> {
    let mut cursor = 2_usize; // past `P4`
    let (width, advanced) = read_pbm_uint_after_whitespace(&bytes[cursor..])?;
    cursor += advanced;
    let (height, advanced) = read_pbm_uint_after_whitespace(&bytes[cursor..])?;
    cursor += advanced;
    let first_pixel_byte = *bytes.get(cursor)?;
    if !first_pixel_byte.is_ascii_whitespace() {
        return None;
    }
    cursor += 1;

    let width_us = usize::try_from(width).ok()?;
    let height_us = usize::try_from(height).ok()?;
    let stride = width_us.div_ceil(8);
    let expected = stride.checked_mul(height_us)?;
    if bytes.len() < cursor + expected {
        return None;
    }

    // Take ownership of just the pixel bytes by draining the header off the
    // front. This avoids allocating + copying a fresh `Vec` for the body.
    bytes.drain(0..cursor);
    bytes.truncate(expected);

    Some(BitPage::from_packed(bytes, width, height))
}

fn read_pbm_uint_after_whitespace(s: &[u8]) -> Option<(u32, usize)> {
    let mut i = 0;
    loop {
        let b = *s.get(i)?;
        if b == b'#' {
            while i < s.len() && s[i] != b'\n' {
                i += 1;
            }
        } else if b.is_ascii_whitespace() {
            i += 1;
        } else {
            break;
        }
    }
    let start = i;
    let mut value: u32 = 0;
    while let Some(&b) = s.get(i) {
        if !b.is_ascii_digit() {
            break;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(b - b'0'))?;
        i += 1;
    }
    if i == start {
        return None;
    }
    Some((value, i))
}

/// Pack an 8-bit grayscale buffer (caller asserts every pixel is `0` or
/// `255`) into a `BitPage`. Uses the `pmovmskb` AVX2 helper from
/// [`crate::bitplane::pack_row`].
fn bit_pack_grayscale(gray: &GrayImage) -> BitPage {
    let width = gray.width();
    let height = gray.height();
    let stride = (width as usize).div_ceil(8);
    let mut bits = vec![0_u8; stride * height as usize];
    let src = gray.as_raw();
    let w = width as usize;
    for y in 0..height as usize {
        let src_row = &src[y * w..(y + 1) * w];
        let dest_row = &mut bits[y * stride..(y + 1) * stride];
        crate::bitplane::pack_row(src_row, dest_row, w);
    }
    BitPage::from_packed(bits, width, height)
}

fn is_effectively_bitonal(img: &GrayImage) -> bool {
    img.pixels().all(|p| matches!(p.0[0], 0 | 255))
}
