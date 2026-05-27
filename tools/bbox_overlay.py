r"""Render every page's text-block bbox onto one canvas.

The cleanest single-image proof that register-rs actually aligned anything.
For each page in BOTH the original and the registered directory, compute
the ink-pixel bounding box and draw it as a thin rectangle outline on a
single shared canvas. Original bboxes are red, registered bboxes are
green. Stack all of them on top of each other.

What you see at a glance:

- **Loose red cloud** of slightly-displaced rectangles → that's the
  per-page jitter in the original scan corpus.
- **Single sharp green rectangle** (or a tight green cluster) on top →
  every registered page lands at the same canvas position.

If alignment worked: many red rectangles, one green rectangle. If
something went wrong: red and green clouds look the same shape.

Original bboxes are computed on each page *centered on the registered
canvas* (i.e. the "do-nothing" baseline), so the comparison is fair.

Usage:
    python tools/bbox_overlay.py \\
        --before private/extracted/russell \\
        --after  private/extracted/russell-out \\
        --output artifacts/russell-bbox.png
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import numpy as np
from PIL import Image, ImageDraw

if TYPE_CHECKING:
    from collections.abc import Iterator


def iter_pairs(orig_dir: Path, registered_dir: Path) -> Iterator[tuple[Path, Path]]:
    """Yield filename-matched (orig, registered) pairs."""
    for orig in sorted(p for p in orig_dir.iterdir() if p.is_file()):
        candidate = registered_dir / orig.name
        if candidate.is_file():
            yield orig, candidate


def ink_bbox(arr: np.ndarray) -> tuple[int, int, int, int] | None:
    """Return `(x0, y0, x1, y1)` of the ink-pixel bbox, or `None` if no ink.

    `arr` is a 2D u8 grayscale array with 0 = ink. Returned coordinates use
    the exclusive-bottom-right convention (so `x1 - x0` is the bbox width).
    """
    ink = arr == 0
    if not ink.any():
        return None
    rows = np.where(ink.any(axis=1))[0]
    cols = np.where(ink.any(axis=0))[0]
    y0, y1 = int(rows[0]), int(rows[-1]) + 1
    x0, x1 = int(cols[0]), int(cols[-1]) + 1
    return x0, y0, x1, y1


def main() -> int:
    """CLI entry point."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument("--before", type=Path, required=True, help="unprocessed corpus dir")
    parser.add_argument("--after", type=Path, required=True, help="registered corpus dir")
    parser.add_argument("--output", type=Path, required=True, help="output PNG path")
    args = parser.parse_args()

    registered_pages = sorted(p for p in args.after.iterdir() if p.is_file())
    if not registered_pages:
        print(f"error: no pages in {args.after}", file=sys.stderr)
        return 1
    canvas_w, canvas_h = Image.open(registered_pages[0]).size

    # White canvas; draw red original bboxes then green registered bboxes
    # on top. Use a thin line so the lower-density edges of the red cloud
    # don't drown out the green box.
    canvas = Image.new("RGB", (canvas_w, canvas_h), (255, 255, 255))
    draw = ImageDraw.Draw(canvas)

    counted = 0
    for orig_path, registered_path in iter_pairs(args.before, args.after):
        # ORIGINAL: center on canvas (the no-alignment baseline) and bbox
        orig_img = Image.open(orig_path).convert("L")
        centered = Image.new("L", (canvas_w, canvas_h), 255)
        px = (canvas_w - orig_img.width) // 2
        py = (canvas_h - orig_img.height) // 2
        centered.paste(orig_img, (px, py))
        before_bb = ink_bbox(np.asarray(centered, dtype=np.uint8))
        if before_bb is not None:
            x0, y0, x1, y1 = before_bb
            draw.rectangle((x0, y0, x1 - 1, y1 - 1), outline=(255, 0, 0, 30), width=1)

        # REGISTERED: take bbox directly
        registered_arr = np.asarray(Image.open(registered_path).convert("L"), dtype=np.uint8)
        after_bb = ink_bbox(registered_arr)
        if after_bb is not None:
            x0, y0, x1, y1 = after_bb
            draw.rectangle((x0, y0, x1 - 1, y1 - 1), outline=(0, 180, 0, 30), width=1)

        counted += 1
        if counted % 50 == 0:
            print(f"  drew {counted} bbox pairs…", file=sys.stderr)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(args.output, optimize=True)
    print(f"wrote {args.output} ({canvas.size[0]}x{canvas.size[1]}, {counted} pages)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
