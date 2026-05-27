# register-rs

Align bitonal scans of Japanese novel pages onto a fixed paper-size canvas.

`register-rs` is one piece of a Rust pipeline that post-processes self-scanned
PDF books so every page looks visually aligned. This crate focuses on a
single problem: every scan of the same book produces slightly different page
sizes, text-block positions, and rotations, and `register-rs` removes that
jitter by detecting each page's main column, deriving a corpus-wide reference
layout per parity (recto/verso), and rendering every page onto the same
exact-size canvas — typically ISO B5.

Status: pre-release scaffold. The pipeline runs end-to-end at translation-only
fidelity; scale normalization and skew correction land in subsequent versions.

## Boundaries

- **Input / Output**: a directory of 1-bit images (PBM/PNG/TIFF) in,
  same-extension 1-bit images out. `register-rs` never touches PDF containers
  itself — pair it with `pdfimages` on the way in and an image→PDF re-packer
  on the way out. This mirrors the
  [`despeckle`](https://github.com/P4suta/despeckle) boundary so the two
  tools chain cleanly:

  ```text
  pdfimages → despeckle (noise removal) → register-rs (alignment) → image→PDF re-packer
  ```

- **Novels only, vertical typesetting**: parity handling assumes a Japanese
  right-to-left binding and a single main text column. Tables, figures, and
  horizontal layouts are out of scope.
- **No tuning UI**: the only paper / DPI knobs exist because the output
  canvas must be a number; everything else is derived from the corpus.

## Quick start

```sh
just bootstrap          # build the dev container, install git hooks
just build              # cargo build via the dev container
just run-sample         # process samples/ → artifacts/sample-out
```

## Architecture in one diagram

```text
  RawPage ─analyze─▶ AnalyzedPage ─plan─▶ AlignmentPlan ─render─▶ RegisteredPage
                                       ╲                       ╱
                                        ╲─── Passthrough ────╱       (analysis failed)
```

Each arrow is a pure function in `register-core`. The CLI (`register-cli`)
owns I/O, walking the input directory, parallelism, and progress reporting.

`raster::render` is the **sole** site that may resample (rotation, scale).
Translation-only renderings stay losslessly bitonal; destructive operations
are confined to one module so future versions can change re-binarisation
strategy in one place.

## License

Dual-licensed under MIT or Apache-2.0 at your option.
