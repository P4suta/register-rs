//! End-to-end smoke for the `register` binary.
//!
//! Synthesizes a small corpus of 1-bit PBM pages, runs the CLI against it
//! at a low DPI (so the canvas stays small enough for a fast test), and
//! verifies that every output is exactly the expected canvas size.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration tests legitimately panic on assertion failure"
)]

use std::path::Path;

use assert_cmd::Command;
use image::{GrayImage, Luma};

fn write_dot(path: &Path, w: u32, h: u32, dot_x: u32, dot_y: u32) {
    let mut img = GrayImage::from_pixel(w, h, Luma([255]));
    img.put_pixel(dot_x, dot_y, Luma([0]));
    img.save(path).expect("write PBM");
}

#[test]
fn end_to_end_aligns_every_page_to_canvas_size() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let input = tmp.path().join("in");
    let output = tmp.path().join("out");
    std::fs::create_dir_all(&input).expect("mkdir in");

    // 5 synthetic 80×100 pages, each with a single black dot at a slightly
    // different position. Page sort order = corpus order; the dot positions
    // jitter the way real scans might.
    for (i, (dx, dy)) in [(20, 30), (22, 31), (18, 29), (21, 32), (19, 30)]
        .iter()
        .enumerate()
    {
        let name = format!("page-{i:02}.pbm");
        write_dot(&input.join(&name), 80, 100, *dx, *dy);
    }

    Command::cargo_bin("register")
        .expect("binary exists")
        .arg(&input)
        .arg(&output)
        .args(["--paper", "b5", "--dpi", "50"])
        .assert()
        .success();

    // ISO B5 at 50 DPI = round(182 * 50 / 25.4) × round(257 * 50 / 25.4) = 358 × 506.
    for entry in std::fs::read_dir(&output).expect("read out") {
        let entry = entry.expect("dir entry");
        let img = image::open(entry.path()).expect("decode output").to_luma8();
        assert_eq!(img.dimensions(), (358, 506), "{}", entry.path().display());
        // bitonal invariant must hold on output.
        assert!(
            img.pixels().all(|p| matches!(p.0[0], 0 | 255)),
            "non-bitonal pixel in {}",
            entry.path().display()
        );
    }
}
