"""Render a before/after overlay PDF showing what register-rs aligned.

Given two parallel directories — the original scan output and the registered
output — produces a single PDF where every page is a color-coded composite:

- **Black**  pixels that exist in both the original and the registered page
  (ink that landed at the same canvas coordinate either way).
- **Red**    ink that existed in the original but not the registered page —
  pixels that got shifted out of this position by the alignment.
- **Green**  ink in the registered page but not the original — pixels that
  the alignment shifted INTO this position.
- **White**  paper (neither original nor registered has ink here).

In the diff PDF you should see the same text block at roughly the same canvas
position on every page (because registered pages all sit at the corpus's
median text-block position), with the per-page registration shift visualized
as red↔green ghosting at the edges of every glyph. A page with NO red /
green means the source already happened to align — useful as a sanity check
that the tool doesn't move pages it shouldn't.

The "original" image is positioned on the canvas by simple centering (i.e.
the no-alignment baseline), so the diff isolates exactly what the alignment
step did.

Usage:
    python tools/overlay_diff.py <orig_dir> <registered_dir> <out.pdf>
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import TYPE_CHECKING

from PIL import Image, ImageChops

if TYPE_CHECKING:
    from collections.abc import Iterator


def iter_pairs(orig_dir: Path, registered_dir: Path) -> Iterator[tuple[Path, Path]]:
    """Yield matching `(orig, registered)` page-file pairs in sort order.

    Pages are matched purely by filename: register-rs mirrors the input
    layout one-to-one, so `russell/page-01.pbm` always pairs with
    `russell-out/page-01.pbm`.
    """
    orig_files = sorted(p for p in orig_dir.iterdir() if p.is_file())
    for orig in orig_files:
        candidate = registered_dir / orig.name
        if candidate.is_file():
            yield orig, candidate


def overlay_one(orig_path: Path, registered_path: Path) -> Image.Image:
    """Build the RGB diff composite for one page.

    Per-channel logic (with `ink = 0`, `white = 255`):
    - R channel = the registered page's grayscale (white where no ink)
    - G channel = the centered original's grayscale
    - B channel = pixel-wise `min(R, G)` so both-ink stays black and
      either-only stays in a primary color.
    """
    original = Image.open(orig_path).convert("L")
    registered = Image.open(registered_path).convert("L")

    # Center the original on a canvas the size of the registered page so the
    # two grayscale layers line up coordinate-for-coordinate.
    canvas_w, canvas_h = registered.size
    centered_original = Image.new("L", (canvas_w, canvas_h), 255)
    paste_x = (canvas_w - original.width) // 2
    paste_y = (canvas_h - original.height) // 2
    centered_original.paste(original, (paste_x, paste_y))

    # Compose: R = registered, G = original, B = min(R, G).
    blue = ImageChops.darker(registered, centered_original)
    return Image.merge("RGB", (registered, centered_original, blue))


def render_overlay_pdf(
    orig_dir: Path,
    registered_dir: Path,
    out_pdf: Path,
    downsample: int,
) -> int:
    """Render the per-page overlay images and pack them into a single PDF.

    Returns the number of pages emitted.
    """
    if not orig_dir.is_dir():
        print(f"error: original directory not found: {orig_dir}", file=sys.stderr)
        return 0
    if not registered_dir.is_dir():
        print(f"error: registered directory not found: {registered_dir}", file=sys.stderr)
        return 0

    # Build composite images in a temp dir, then hand the lot to img2pdf in
    # one go. img2pdf wants files on disk, not in-memory streams.
    with tempfile.TemporaryDirectory(prefix="register-overlay-") as tmp:
        tmp_path = Path(tmp)
        page_paths: list[Path] = []
        for n, (orig, registered) in enumerate(iter_pairs(orig_dir, registered_dir), start=1):
            composite = overlay_one(orig, registered)
            if downsample > 1:
                composite = composite.reduce(downsample)
            page_path = tmp_path / f"{n:05d}.png"
            composite.save(page_path, "PNG", optimize=True)
            page_paths.append(page_path)
            if n % 50 == 0:
                print(f"  composed {n} pages…", file=sys.stderr)

        if not page_paths:
            print("error: no matching page pairs found", file=sys.stderr)
            return 0

        out_pdf.parent.mkdir(parents=True, exist_ok=True)
        # `img2pdf` packs ONGs without re-encoding their pixel data.
        cmd = ["img2pdf", "--output", str(out_pdf), *[str(p) for p in page_paths]]
        subprocess.run(cmd, check=True)
        return len(page_paths)


def main() -> int:
    """CLI entry point."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument("orig_dir", type=Path, help="directory of unprocessed PBM/PNG pages")
    parser.add_argument(
        "registered_dir",
        type=Path,
        help="directory of pages produced by `register`",
    )
    parser.add_argument("out_pdf", type=Path, help="path to write the overlay PDF to")
    parser.add_argument(
        "--downsample",
        type=int,
        default=2,
        help=(
            "downsample factor for the composite images (default: 2, "
            "≈ half the original resolution; use 1 for full)."
        ),
    )
    args = parser.parse_args()

    written = render_overlay_pdf(
        args.orig_dir,
        args.registered_dir,
        args.out_pdf,
        args.downsample,
    )
    if written == 0:
        return 1
    print(f"wrote {written} pages → {args.out_pdf}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
