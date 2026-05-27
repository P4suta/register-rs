r"""Per-page delta map: show ONLY what register-rs moved.

Run on the (before-dir, after-dir) pair from a register-rs corpus run.
For every (original, registered) page pair the output PDF contains one page
that is white **except** at pixels where the alignment changed something:

- **Red**  ink that was in the original at this canvas position but the
  registered page no longer has — pixels register moved AWAY from here.
- **Green** ink the registered page has at this position but the original
  didn't — pixels register moved IN to here.

Where both before and after have ink (text that didn't move), and where
neither does (margin), the result is white — so the diff is *just* the
delta. This is the easiest way to spot-check "is register doing the right
thing? did it move pages it shouldn't, or skip ones it should?".

The "original" image is positioned on the canvas by simple centering
(register's no-alignment baseline), so what you see is exactly the shift
register produced relative to "do nothing".

Heavy lifting goes through numpy: each page composite is ~6 MP on B5/300
DPI and pure-Python pixel loops are unusable. With numpy the per-page cost
is bound by memory bandwidth on the comparison + the PNG encoder.

Usage:
    python tools/diff_pages.py \\
        --before private/extracted/russell \\
        --after  private/extracted/russell-out \\
        --output artifacts/russell-delta.pdf
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import TYPE_CHECKING

import numpy as np
from PIL import Image

if TYPE_CHECKING:
    from collections.abc import Iterator


def iter_pairs(orig_dir: Path, registered_dir: Path) -> Iterator[tuple[Path, Path]]:
    """Yield filename-matched `(orig, registered)` page pairs in sort order."""
    for orig in sorted(p for p in orig_dir.iterdir() if p.is_file()):
        candidate = registered_dir / orig.name
        if candidate.is_file():
            yield orig, candidate


def delta_one(orig_path: Path, registered_path: Path) -> Image.Image:
    """Build the delta-only RGB image for one page pair.

    Center the original on the registered page's canvas, then mark
    every pixel where exactly one side has ink:

    - `before_ink & ~after_ink` → red  (255, 0, 0)
    - `~before_ink & after_ink` → green (0, 255, 0)
    - otherwise                  → white (255, 255, 255)
    """
    original_img = Image.open(orig_path).convert("L")
    registered_img = Image.open(registered_path).convert("L")

    canvas_w, canvas_h = registered_img.size
    centered = Image.new("L", (canvas_w, canvas_h), 255)
    px = (canvas_w - original_img.width) // 2
    py = (canvas_h - original_img.height) // 2
    centered.paste(original_img, (px, py))

    before = np.asarray(centered, dtype=np.uint8)
    after = np.asarray(registered_img, dtype=np.uint8)
    # `ink = pixel == 0`; bool arrays make the boolean logic obvious.
    before_ink = before == 0
    after_ink = after == 0
    lost = before_ink & ~after_ink  # red
    gained = ~before_ink & after_ink  # green

    rgb = np.full((canvas_h, canvas_w, 3), 255, dtype=np.uint8)
    rgb[lost] = (255, 0, 0)
    rgb[gained] = (0, 255, 0)
    return Image.fromarray(rgb, mode="RGB")


def render_delta_pdf(
    orig_dir: Path,
    registered_dir: Path,
    out_pdf: Path,
    downsample: int,
) -> int:
    """Generate per-page delta images and pack them into a single PDF.

    Returns the number of pages emitted.
    """
    if not orig_dir.is_dir():
        print(f"error: original directory not found: {orig_dir}", file=sys.stderr)
        return 0
    if not registered_dir.is_dir():
        print(f"error: registered directory not found: {registered_dir}", file=sys.stderr)
        return 0

    with tempfile.TemporaryDirectory(prefix="register-delta-") as tmp:
        tmp_path = Path(tmp)
        page_paths: list[Path] = []
        for n, (orig, registered) in enumerate(iter_pairs(orig_dir, registered_dir), start=1):
            composite = delta_one(orig, registered)
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
        cmd = ["img2pdf", "--output", str(out_pdf), *[str(p) for p in page_paths]]
        subprocess.run(cmd, check=True)
        return len(page_paths)


def main() -> int:
    """CLI entry point."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument("--before", type=Path, required=True, help="unprocessed corpus dir")
    parser.add_argument("--after", type=Path, required=True, help="registered corpus dir")
    parser.add_argument("--output", type=Path, required=True, help="output PDF path")
    parser.add_argument(
        "--downsample",
        type=int,
        default=2,
        help="reduce composite by this factor (default 2; use 1 for full)",
    )
    args = parser.parse_args()

    written = render_delta_pdf(args.before, args.after, args.output, args.downsample)
    if written == 0:
        return 1
    print(f"wrote {written} pages → {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
