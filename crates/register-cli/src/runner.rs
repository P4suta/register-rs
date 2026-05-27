//! Directory walker + streaming pipeline driver.
//!
//! Architecture (root-solution multi-threading):
//!
//! Instead of two parallel phases separated by a barrier ("analyze all →
//! derive reference → render all"), every page flows through *one* pipeline
//! pinned to a worker thread:
//!
//! ```text
//!   load  →  analyze  →  submit bbox  →  wait for reference  →  plan + render + save
//! ```
//!
//! A small **coordinator** collects the first `REFERENCE_THRESHOLD` bboxes
//! that arrive, computes the per-parity median reference, and broadcasts
//! it to every worker via a `Condvar`. From that point on, every other page
//! reads the already-cached reference without blocking — so the I/O for
//! loading new pages overlaps with rendering older ones.
//!
//! Wins over the previous two-phase rayon design:
//!
//! - The phase barrier (≈ 50 ms wall on a 343-page B5 corpus) is gone.
//! - Workers are continuously busy: there is no "phase 1 finishes, phase 2
//!   starts" dead time.
//! - Memory footprint is bounded by `num_workers × per_page_size` instead
//!   of `num_pages × per_page_size`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::{Context, Result};
use crossbeam_channel as ch;
use globset::{Glob, GlobMatcher};
use indicatif::{ProgressBar, ProgressStyle};
use register_core::{
    BoundingBox, CanvasBuf, Paper, Parity, PlanOptions, ReferenceLayout, TargetCanvas, analyze,
    derive_reference_from_snapshots, load_bitonal, plan_alignment, plan_passthrough, render_into,
    save_bitonal,
};
use walkdir::WalkDir;

use crate::cli::{Args, OutputFormat};

/// Pages whose bboxes feed the reference, expressed as a multiplier on the
/// worker count. We want enough samples for a stable per-parity median
/// (≥ 8 — the per-page jitter on a uniform-content book is well below
/// the bbox size) but **not more than `num_workers`**, since every
/// worker holds onto its analyzed page while waiting for the broadcast:
/// asking for more bboxes than there are workers in flight would deadlock.
const REFERENCE_THRESHOLD_PER_WORKER: usize = 1;
const REFERENCE_THRESHOLD_MIN: usize = 8;

/// Drive the full register-rs pipeline against `args`.
pub(crate) fn run(args: &Args) -> Result<()> {
    prepare_output_dir(&args.output_dir, args.force)?;

    let matcher = compile_glob(&args.glob)?;
    let files = collect_files(&args.input_dir, &matcher);
    if files.is_empty() {
        tracing::warn!("no images matched in input directory");
        return Ok(());
    }

    let paper: Paper = args.paper_mm.map_or_else(
        || args.paper.into(),
        |(width_mm, height_mm)| Paper::Custom {
            width_mm,
            height_mm,
        },
    );
    let canvas = paper.canvas_at_dpi(args.dpi);
    let plan_options = PlanOptions {
        scale: !args.no_scale,
        skew: !args.no_skew,
    };
    let num_workers = args
        .jobs
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, usize::from));

    tracing::info!(
        input = %args.input_dir.display(),
        output = %args.output_dir.display(),
        pages = files.len(),
        canvas_px = format!("{}x{}", canvas.width, canvas.height),
        dpi = canvas.dpi,
        workers = num_workers,
        "starting register"
    );

    let bar = build_progress_bar(files.len(), "registering")?;
    let threshold = (num_workers * REFERENCE_THRESHOLD_PER_WORKER)
        .max(REFERENCE_THRESHOLD_MIN)
        .min(num_workers)
        .min(files.len().max(1));
    let coordinator = Arc::new(Coordinator::new(threshold, canvas));

    // Bounded channel — backpressure prevents a fast loader from racing
    // ahead of the render+save workers and exploding memory.
    let (work_tx, work_rx) = ch::bounded::<(usize, PathBuf)>(num_workers * 2);

    std::thread::scope(|s| -> Result<()> {
        // Producer
        let producer_files = files.clone();
        let producer = s.spawn(move || -> Result<()> {
            for (i, path) in producer_files.into_iter().enumerate() {
                work_tx
                    .send((i, path))
                    .context("worker pool dropped before producer finished")?;
            }
            Ok(())
        });

        // Workers
        let mut handles = Vec::with_capacity(num_workers);
        for _ in 0..num_workers {
            let work_rx = work_rx.clone();
            let coord = Arc::clone(&coordinator);
            let bar = bar.clone();
            let args_ref = &*args;
            handles.push(s.spawn(move || -> Result<()> {
                let mut buf = CanvasBuf::new(canvas);
                for (i, path) in work_rx {
                    process_one(i, &path, &coord, plan_options, canvas, args_ref, &mut buf)?;
                    bar.inc(1);
                }
                Ok(())
            }));
        }

        // Spin until producer finishes (so we can join its result before
        // the workers drop their senders).
        producer.join().expect("producer thread panicked")?;
        // Coordinator can also be the "kicker" if fewer than the threshold of
        // pages ever arrive — in that case nobody triggers the broadcast.
        coordinator.kick_if_short_corpus(files.len());

        for h in handles {
            h.join().expect("worker thread panicked")?;
        }
        bar.finish_and_clear();
        Ok(())
    })?;

    tracing::info!(pages = files.len(), "register done");
    Ok(())
}

fn process_one(
    index: usize,
    path: &Path,
    coord: &Coordinator,
    plan_options: PlanOptions,
    canvas: TargetCanvas,
    args: &Args,
    buf: &mut CanvasBuf,
) -> Result<()> {
    let raw =
        load_bitonal(path, index).with_context(|| format!("failed to load {}", path.display()))?;
    let analyzed_or_raw = analyze(raw);

    // Submit bbox to the coordinator, then wait until the reference is
    // ready. Pages that don't have ink (`Err(raw)` from `analyze`) still
    // wait on the same broadcast — they need the reference for the
    // canvas's known center.
    let bbox_snapshot = analyzed_or_raw
        .as_ref()
        .ok()
        .map(|a| (a.parity(), a.layout().main_column));
    let reference = coord.submit_and_wait(bbox_snapshot)?;

    let plan = match analyzed_or_raw {
        Ok(a) => plan_alignment(a, &reference, plan_options),
        Err(r) => plan_passthrough(r),
    };

    let dest = mirror_destination(path, &args.input_dir, &args.output_dir, args.format)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    render_into(plan, canvas, buf);
    save_bitonal(buf.bits(), &dest)
        .with_context(|| format!("failed to save {}", dest.display()))?;
    Ok(())
}

/// Reference-broadcast coordinator. Workers send their bbox once analysis
/// finishes; the coordinator computes the per-parity median reference once
/// `threshold` bboxes have arrived and signals every worker via a Condvar.
struct Coordinator {
    state: Mutex<CoordState>,
    ready: Condvar,
    threshold: usize,
    canvas: TargetCanvas,
}

struct CoordState {
    /// `(Parity, bbox)` snapshots from workers. We only ever read the first
    /// `threshold` entries — late submissions are accepted but ignored.
    snapshots: Vec<(Parity, BoundingBox)>,
    /// Some(_) once `threshold` bboxes have arrived and the reference is
    /// committed.
    reference: Option<Arc<ReferenceLayout>>,
    /// Count of workers that have called `submit_and_wait` (including those
    /// with no bbox — passthrough pages contribute nothing to the median).
    submitted: usize,
    /// Total pages we expect; set by the producer once all paths are queued.
    /// Until then `submit_and_wait` only triggers via the `threshold` path.
    submitted_target: Option<usize>,
}

impl Coordinator {
    fn new(threshold: usize, canvas: TargetCanvas) -> Self {
        Self {
            state: Mutex::new(CoordState {
                snapshots: Vec::with_capacity(threshold * 2),
                reference: None,
                submitted: 0,
                submitted_target: None,
            }),
            ready: Condvar::new(),
            threshold,
            canvas,
        }
    }

    /// Append `bbox` to the coordinator's pool (if `Some`) and block until
    /// the reference layout is broadcast. Returns the broadcast reference.
    ///
    /// The first thread to push the threshold-th bbox computes and stores
    /// the reference, then notifies everyone.
    fn submit_and_wait(&self, bbox: Option<(Parity, BoundingBox)>) -> Result<Arc<ReferenceLayout>> {
        let mut state = self.state.lock().expect("coordinator mutex poisoned");
        state.submitted += 1;
        if let Some(snap) = bbox {
            state.snapshots.push(snap);
        }
        let should_trigger = state.reference.is_none()
            && (state.snapshots.len() >= self.threshold
                // Last-resort kick for short corpora: every page has been
                // submitted and we still don't have `threshold` bboxes,
                // either because the corpus is small or every page was
                // blank. Use whatever we have (possibly nothing — then the
                // coordinator falls back to a centered identity reference).
                || state.submitted == state.submitted_target.unwrap_or(usize::MAX));
        if should_trigger {
            let reference = compute_reference_or_fallback(&state.snapshots, self.canvas)?;
            state.reference = Some(Arc::new(reference));
            self.ready.notify_all();
        }
        while state.reference.is_none() {
            state = self
                .ready
                .wait(state)
                .expect("coordinator condvar wait failed");
        }
        Ok(Arc::clone(
            state.reference.as_ref().expect("reference set above"),
        ))
    }

    /// Called after the producer has handed every path to the workers.
    /// Stashes the total submission count so the next-arriving worker
    /// triggers the broadcast even if the bbox pool never reaches
    /// `threshold` (small corpora, all-blank pages).
    fn kick_if_short_corpus(&self, total_pages: usize) {
        // Take the mutex only as long as we mutate the shared state; the
        // `notify_all` happens after the guard is dropped so waking workers
        // don't contend for the lock we no longer need.
        let to_broadcast = {
            let mut state = self.state.lock().expect("coordinator mutex poisoned");
            state.submitted_target = Some(total_pages);
            if state.reference.is_some() || state.submitted < total_pages {
                None
            } else if let Ok(reference) =
                compute_reference_or_fallback(&state.snapshots, self.canvas)
            {
                let arc = Arc::new(reference);
                state.reference = Some(Arc::clone(&arc));
                Some(arc)
            } else {
                None
            }
        };
        if to_broadcast.is_some() {
            self.ready.notify_all();
        }
    }
}

fn compute_reference_or_fallback(
    snapshots: &[(Parity, BoundingBox)],
    canvas: TargetCanvas,
) -> Result<ReferenceLayout> {
    if snapshots.is_empty() {
        // Whole corpus failed analysis. Fall back to an identity reference
        // (centered bbox of zero size) so passthrough pages still render.
        return Ok(ReferenceLayout {
            canvas,
            recto: BoundingBox {
                x: canvas.width / 2,
                y: canvas.height / 2,
                width: 0,
                height: 0,
            },
            verso: BoundingBox {
                x: canvas.width / 2,
                y: canvas.height / 2,
                width: 0,
                height: 0,
            },
        });
    }
    derive_reference_from_snapshots(snapshots, canvas).context("failed to derive reference")
}

fn prepare_output_dir(dir: &Path, force: bool) -> Result<()> {
    if dir.exists() {
        let non_empty = std::fs::read_dir(dir)
            .with_context(|| format!("failed to read output directory {}", dir.display()))?
            .next()
            .is_some();
        if non_empty && !force {
            anyhow::bail!(
                "output directory {} is non-empty; pass --force to overwrite",
                dir.display()
            );
        }
    } else {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create output directory {}", dir.display()))?;
    }
    Ok(())
}

fn compile_glob(pattern: &str) -> Result<GlobMatcher> {
    let glob = Glob::new(pattern).with_context(|| format!("invalid glob pattern: {pattern}"))?;
    Ok(glob.compile_matcher())
}

fn collect_files(root: &Path, matcher: &GlobMatcher) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .is_some_and(|name| matcher.is_match(name))
        })
        .map(walkdir::DirEntry::into_path)
        .collect();
    files.sort();
    files
}

fn build_progress_bar(total: usize, msg: &str) -> Result<ProgressBar> {
    let style =
        ProgressStyle::with_template("{elapsed_precise} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .context("invalid progress bar template")?
            .progress_chars("=>-");
    Ok(ProgressBar::new(total as u64)
        .with_style(style)
        .with_message(msg.to_string()))
}

fn mirror_destination(
    src: &Path,
    input_root: &Path,
    output_root: &Path,
    format: OutputFormat,
) -> Result<PathBuf> {
    let rel = src
        .strip_prefix(input_root)
        .with_context(|| format!("file outside input root: {}", src.display()))?;
    let mut dest = output_root.join(rel);
    match format {
        OutputFormat::Same => {},
        OutputFormat::Pbm => {
            dest.set_extension("pbm");
        },
        OutputFormat::Png => {
            dest.set_extension("png");
        },
    }
    Ok(dest)
}
