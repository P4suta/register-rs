//! Baseline microbenchmarks for the registration pipeline.
//!
//! Two representative sizes are exercised:
//!
//! - **mid** — 1000 × 1400 px (~B5 at 140 DPI). Fast feedback loop during
//!   iteration.
//! - **full** — 2866 × 4047 px (B5 at 400 DPI, the production target). Sets
//!   the bar that optimizations have to clear.
//!
//! All inputs are synthesized: a centered text-block-shaped black region on
//! a white background, with a sprinkle of pepper noise. This is a *better*
//! stress test than uniform fills because it exercises both the all-white
//! and the dense-ink fast paths, and the projection / bbox routines must
//! actually look at the pixels.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::missing_docs_in_private_items,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    missing_docs,
    reason = "benchmarks are pure throughput probes; lint policy applies to library code"
)]

use std::path::PathBuf;
use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use imageproc::geometric_transformations::Projection;
use register_core::{
    AlignmentKind, AlignmentPlan, AnalyzedPage, BitPage, Paper, PlanOptions, RawPage, TargetCanvas,
    analyze, derive_reference, plan_alignment, render,
};

/// Build a synthetic bit-packed page: a centered text-block-shaped black
/// region (1-bit, MSB-first) with a sprinkle of pepper noise.
fn synth_page(width: u32, height: u32, seed: u64) -> BitPage {
    let mut bp = BitPage::new_white(width, height);
    let block_w = (width as f32 * 0.6) as u32;
    let block_h = (height as f32 * 0.75) as u32;
    let jitter = (seed % 21) as i32 - 10;
    let block_x = (width / 2)
        .saturating_sub(block_w / 2)
        .saturating_add_signed(jitter);
    let block_y = (height / 2)
        .saturating_sub(block_h / 2)
        .saturating_add_signed(jitter / 2);

    for y in block_y..block_y.saturating_add(block_h).min(height) {
        if !y.is_multiple_of(2) {
            continue;
        }
        for x in block_x..block_x.saturating_add(block_w).min(width) {
            if x % 3 == 0 {
                set_bit(&mut bp, x, y);
            }
        }
    }
    let mut rng = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    for _ in 0..256 {
        rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let x = (rng >> 32) as u32 % width;
        let y = (rng >> 16) as u32 % height;
        set_bit(&mut bp, x, y);
    }
    bp
}

fn set_bit(bp: &mut BitPage, x: u32, y: u32) {
    let stride = bp.stride();
    let row = bp.bytes_mut();
    row[y as usize * stride + (x as usize / 8)] |= 1 << (7 - (x % 8));
}

fn synth_raw(width: u32, height: u32, index: usize) -> RawPage {
    let bits = synth_page(width, height, index as u64);
    register_core::test_only::raw_from_bits(
        bits,
        PathBuf::from(format!("synth-{index}.pbm")),
        index,
    )
}

fn bench_analyze(c: &mut Criterion) {
    for &(name, w, h) in &[("mid", 1000_u32, 1400_u32), ("full", 2866_u32, 4047_u32)] {
        let mut group = c.benchmark_group(format!("analyze/{name}"));
        group.throughput(Throughput::Elements(u64::from(w) * u64::from(h)));
        group.measurement_time(Duration::from_secs(5));
        group.bench_function("ink_bbox", |b| {
            b.iter_batched(
                || synth_raw(w, h, 0),
                |raw| analyze(raw).expect("ink present"),
                BatchSize::LargeInput,
            );
        });
        group.finish();
    }
}

fn bench_render(c: &mut Criterion) {
    for &(name, w, h) in &[("mid", 1000_u32, 1400_u32), ("full", 2866_u32, 4047_u32)] {
        let canvas = TargetCanvas {
            width: w,
            height: h,
            dpi: 400,
        };
        let mut group = c.benchmark_group(format!("render/{name}"));
        group.throughput(Throughput::Elements(u64::from(w) * u64::from(h)));
        group.measurement_time(Duration::from_secs(5));

        // Aligned + Translate (integer translation, lossless fast path).
        group.bench_function("aligned_translate", |b| {
            let raw = synth_raw(w, h, 0);
            let analyzed = analyze(raw).expect("ink present");
            b.iter_batched(
                || AlignmentPlan::Aligned {
                    analyzed: analyzed.clone(),
                    kind: AlignmentKind::Translate { dx: 5, dy: 7 },
                },
                |plan| render(plan, canvas),
                BatchSize::LargeInput,
            );
        });

        // Aligned + Affine (warp_into + rebinarise — destructive slow path).
        group.bench_function("aligned_affine", |b| {
            let raw = synth_raw(w, h, 0);
            let analyzed = analyze(raw).expect("ink present");
            let transform = Projection::translate(5.0, 7.0);
            b.iter_batched(
                || AlignmentPlan::Aligned {
                    analyzed: analyzed.clone(),
                    kind: AlignmentKind::Affine(transform),
                },
                |plan| render(plan, canvas),
                BatchSize::LargeInput,
            );
        });

        // Passthrough (centered paste, no resampling) path.
        group.bench_function("passthrough", |b| {
            let raw = synth_raw(w, h, 0);
            b.iter_batched(
                || AlignmentPlan::Passthrough { raw: raw.clone() },
                |plan| render(plan, canvas),
                BatchSize::LargeInput,
            );
        });
        group.finish();
    }
}

fn bench_pipeline(c: &mut Criterion) {
    let pages_per_corpus = 8_usize;
    {
        let (name, w, h) = ("mid", 1000_u32, 1400_u32);
        let canvas = Paper::B5.canvas_at_dpi(140);
        let mut group = c.benchmark_group(format!("pipeline/{name}"));
        group.throughput(Throughput::Elements(pages_per_corpus as u64));
        group.measurement_time(Duration::from_secs(8));
        group.bench_function("analyze+reference+plan+render", |b| {
            b.iter_batched(
                || {
                    (0..pages_per_corpus)
                        .map(|i| synth_raw(w, h, i))
                        .collect::<Vec<_>>()
                },
                |corpus| {
                    let analyzed: Vec<AnalyzedPage> =
                        corpus.into_iter().filter_map(|r| analyze(r).ok()).collect();
                    let reference =
                        derive_reference(analyzed.iter(), canvas).expect("non-empty corpus");
                    let rendered: Vec<_> = analyzed
                        .into_iter()
                        .map(|p| plan_alignment(p, &reference, PlanOptions::default()))
                        .map(|plan| render(plan, canvas))
                        .collect();
                    std::hint::black_box(rendered);
                },
                BatchSize::LargeInput,
            );
        });
        group.finish();
    }
}

fn bench_io(c: &mut Criterion) {
    let tmp = tempfile::tempdir().expect("tempdir");
    for &(name, w, h) in &[("mid", 1000_u32, 1400_u32), ("full", 2866_u32, 4047_u32)] {
        let pbm_path = tmp.path().join(format!("{name}.pbm"));
        let bp = synth_page(w, h, 42);
        register_core::save_bitonal(&bp, &pbm_path).expect("seed PBM");

        let mut group = c.benchmark_group(format!("io/{name}"));
        group.throughput(Throughput::Elements(u64::from(w) * u64::from(h)));
        group.measurement_time(Duration::from_secs(5));

        group.bench_function("load_pbm", |b| {
            b.iter(|| register_core::load_bitonal(&pbm_path, 0).expect("decode"));
        });

        group.bench_function("save_pbm_p4", |b| {
            let out = tmp.path().join(format!("{name}-out.pbm"));
            b.iter(|| register_core::save_bitonal(&bp, &out).expect("encode"));
        });
        group.finish();
    }
}

criterion_group!(
    benches,
    bench_analyze,
    bench_render,
    bench_pipeline,
    bench_io
);
criterion_main!(benches);
