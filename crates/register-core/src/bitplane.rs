//! Bit-packed 1-bit-per-pixel page buffer.
//!
//! The pipeline works almost entirely on bitonal data: scanned 1-bit PBM in,
//! aligned 1-bit PBM out. Promoting that data to `image::GrayImage` (one
//! byte per pixel) costs ~8× the memory and ~8× the bandwidth of every
//! linear pass — and even worse, requires bit-unpack on every load and
//! bit-pack on every save. [`BitPage`] keeps the data in its native
//! PBM-compatible representation throughout, so the load and save paths are
//! one big `read` / `write` syscall apiece with no per-pixel transform.
//!
//! ## Representation
//!
//! - `bits[y * stride + x / 8]` byte holds 8 consecutive pixels.
//! - Within each byte, bit `7 - (x % 8)` (MSB-first) is the pixel — `1` for
//!   black, `0` for white. This matches PBM P4 polarity directly, so the
//!   on-disk and in-memory layouts coincide.
//! - `stride = ceil(width / 8)` bytes per row. Trailing bits of the last
//!   byte of each row (when `width % 8 != 0`) are padding and must be kept
//!   zero by anyone who writes into the buffer.

/// 1-bit-per-pixel bitonal page. `1 = black`, `0 = white`. See module docs.
#[derive(Debug, Clone)]
pub struct BitPage {
    pub(crate) bits: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) stride: usize,
}

impl BitPage {
    /// Allocate a fresh white page of the given dimensions.
    #[must_use]
    pub fn new_white(width: u32, height: u32) -> Self {
        let stride = (width as usize).div_ceil(8);
        let bits = vec![0_u8; stride * height as usize];
        Self {
            bits,
            width,
            height,
            stride,
        }
    }

    /// Construct a `BitPage` from a raw PBM-P4 pixel buffer the caller
    /// already validated for size. `stride` MUST equal `ceil(width / 8)`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `bits.len() != stride * height` or `stride`
    /// doesn't match `ceil(width / 8)`.
    #[must_use]
    pub fn from_packed(bits: Vec<u8>, width: u32, height: u32) -> Self {
        let stride = (width as usize).div_ceil(8);
        debug_assert_eq!(
            stride * height as usize,
            bits.len(),
            "BitPage size mismatch"
        );
        Self {
            bits,
            width,
            height,
            stride,
        }
    }

    /// Pixel width.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Pixel height.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Bytes-per-row stride.
    #[must_use]
    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// Underlying bit buffer (`stride * height` bytes).
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bits
    }

    /// Mutable underlying bit buffer.
    #[must_use]
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bits
    }

    /// Reset every byte to `0` (white). Equivalent to `bytes_mut().fill(0)`.
    pub fn reset_white(&mut self) {
        self.bits.fill(0);
    }

    /// Decompose into the underlying packed buffer + dimensions. Useful
    /// when handing data to a writer that wants ownership.
    #[must_use]
    pub fn into_parts(self) -> (Vec<u8>, u32, u32) {
        (self.bits, self.width, self.height)
    }

    /// Borrow a single row's bytes.
    #[must_use]
    pub fn row(&self, y: u32) -> &[u8] {
        let off = y as usize * self.stride;
        &self.bits[off..off + self.stride]
    }

    /// Mutable borrow of a single row's bytes.
    pub fn row_mut(&mut self, y: u32) -> &mut [u8] {
        let off = y as usize * self.stride;
        &mut self.bits[off..off + self.stride]
    }
}

// ---- pack / unpack helpers shared by io.rs (PNG/TIFF fallback) and
// ---- raster.rs (Affine warp path).

/// Pack one row of 8-bit grayscale (`0` = black, `255` = white) into the
/// PBM bit polarity (`1` = black). Dispatches to AVX2 when available.
#[allow(
    unsafe_code,
    reason = "AVX2 fast path requires unsafe; runtime feature-detect"
)]
pub(crate) fn pack_row(src: &[u8], dest: &mut [u8], width: usize) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            // SAFETY: runtime feature flag was just checked.
            unsafe {
                pack_row_avx2(src, dest, width);
            }
            return;
        }
    }
    pack_row_scalar(src, dest, width);
}

fn pack_row_scalar(src: &[u8], dest: &mut [u8], width: usize) {
    let full_bytes = width / 8;
    let tail = width - full_bytes * 8;
    for (byte_idx, chunk) in src[..full_bytes * 8].chunks_exact(8).enumerate() {
        // `chunks_exact(8)` guarantees an 8-byte chunk; the conversion is
        // infallible and the `unwrap_or` arm is dead code.
        let arr: [u8; 8] = chunk.try_into().unwrap_or([0; 8]);
        dest[byte_idx] = pack_8_pixels_u64(arr);
    }
    if tail > 0 {
        let mut last = 0_u8;
        for (i, &px) in src[full_bytes * 8..].iter().take(tail).enumerate() {
            if px == 0 {
                last |= 1 << (7 - i);
            }
        }
        dest[full_bytes] = last;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(
    unsafe_code,
    reason = "AVX2 intrinsics; runtime feature-detect at caller"
)]
#[expect(
    clippy::cast_sign_loss,
    reason = "movemask result is bit-pattern, sign is meaningless"
)]
unsafe fn pack_row_avx2(src: &[u8], dest: &mut [u8], width: usize) {
    use std::arch::x86_64::{_mm256_loadu_si256, _mm256_movemask_epi8};
    let chunk_pixels = 32;
    let mut src_off = 0;
    let mut dest_off = 0;
    while src_off + chunk_pixels <= width {
        // SAFETY: the loop predicate ensures `src[src_off..src_off+32]` is in bounds.
        let v = unsafe { _mm256_loadu_si256(src.as_ptr().add(src_off).cast()) };
        let movemask = _mm256_movemask_epi8(v) as u32;
        let inverted = !movemask;
        let bytes = inverted.to_le_bytes();
        dest[dest_off] = bytes[0].reverse_bits();
        dest[dest_off + 1] = bytes[1].reverse_bits();
        dest[dest_off + 2] = bytes[2].reverse_bits();
        dest[dest_off + 3] = bytes[3].reverse_bits();
        src_off += chunk_pixels;
        dest_off += 4;
    }
    if src_off < width {
        pack_row_scalar(&src[src_off..], &mut dest[dest_off..], width - src_off);
    }
}

#[inline]
fn pack_8_pixels_u64(p: [u8; 8]) -> u8 {
    let block = u64::from_be_bytes(p);
    let highs = (!block) & 0x8080_8080_8080_8080_u64;
    // The shift positions the 8 gathered sign-bits in the lowest byte; the
    // `as u8` truncation keeps exactly that byte.
    (highs.wrapping_mul(0x0002_0408_1020_4081_u64) >> 56) as u8
}

/// Unpack a `BitPage` byte → 8 grayscale pixels lookup table (256 × 8 B).
pub(crate) fn unpack_table() -> &'static [[u8; 8]; 256] {
    use std::sync::LazyLock;
    static TABLE: LazyLock<[[u8; 8]; 256]> = LazyLock::new(|| {
        let mut table = [[0_u8; 8]; 256];
        for (byte_idx, entry) in table.iter_mut().enumerate() {
            #[expect(clippy::cast_possible_truncation, reason = "0..256 fits in u8")]
            let byte = byte_idx as u8;
            for (bit, slot) in entry.iter_mut().enumerate() {
                let mask = 1_u8 << (7 - bit);
                *slot = if byte & mask == 0 { 255 } else { 0 };
            }
        }
        table
    });
    &TABLE
}
