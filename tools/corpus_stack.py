r"""Stack every page of a corpus into one composite.

The gold-standard visual sanity check for whether register-rs actually
aligned anything. Take the per-pixel **density of ink across the whole
corpus** (i.e. for each canvas pixel, what fraction of pages have a black
pixel there?) and render it as a grayscale heatmap.

- Before registering: pages jitter, so each glyph's edges scatter across a
  few pixels — the stack looks blurry.
- After registering: every page's text block sits at the corpus median, so
  the glyphs line up — the stack looks sharp.

Output side-by-side as a single PNG (and optionally PDF) so the user can
literally see "register made the text sharper across the book".

Usage:
    python tools/corpus_stack.py \\
        --before private/extracted/russell \\
        --after  private/extracted/russell-out \\
        --output artifacts/russell-stack.png
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from PIL import Image


def stack_dir(directory: Path, canvas_size: tuple[int, int]) -> Image.Image:
    """Stack every page in `directory` onto a canvas of `canvas_size`.

    Each page contributes `1 / num_pages` worth of ink density at each
    pixel; the resulting grayscale image is the per-pixel ink density.
    Returned image is mode "L" with 0 = black ink everywhere, 255 = paper
    everywhere.
    """
    files = sorted(p for p in directory.iterdir() if p.is_file())
    if not files:
        msg = f"no pages in {directory}"
        raise ValueError(msg)

    canvas_w, canvas_h = canvas_size
    # We accumulate ink fraction in a float buffer (one byte per pixel
    # exceeds u8 for >255 pages), then scale to u8 at the end.
    accumulator = [0.0] * (canvas_w * canvas_h)
    page_count = 0

    for f in files:
        img = Image.open(f).convert("L")
        if img.size != canvas_size:
            # Center the page on the shared canvas (the "no-alignment"
            # baseline). For the registered dir every page already matches
            # canvas size; for the original dir this puts each scan in the
            # canvas center so the comparison is fair.
            centered = Image.new("L", canvas_size, 255)
            px = (canvas_w - img.width) // 2
            py = (canvas_h - img.height) // 2
            centered.paste(img, (px, py))
            img = centered
        # ink = pixel == 0; contribute 1 to accumulator if ink, 0 otherwise.
        for i, v in enumerate(img.tobytes()):
            if v == 0:
                accumulator[i] += 1.0
        page_count += 1
        if page_count % 25 == 0:
            print(f"  stacked {page_count} pages…", file=sys.stderr)

    # Normalize. ink-density d in [0, 1]; output pixel = 255 * (1 - d)
    # (white where no page had ink, black where every page did).
    n = float(page_count)
    out_bytes = bytearray(
        max(0, min(255, round(255.0 * (1.0 - count / n)))) for count in accumulator
    )
    stacked = Image.frombytes("L", canvas_size, bytes(out_bytes))
    print(f"  done: {page_count} pages from {directory}", file=sys.stderr)
    return stacked


def side_by_side(left: Image.Image, right: Image.Image, gap: int = 24) -> Image.Image:
    """Compose two equal-height stacks side-by-side with a white gap."""
    w = left.width + right.width + gap
    h = max(left.height, right.height)
    out = Image.new("L", (w, h), 255)
    out.paste(left, (0, 0))
    out.paste(right, (left.width + gap, 0))
    return out


def main() -> int:
    """CLI entry point."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument("--before", type=Path, required=True, help="unprocessed corpus dir")
    parser.add_argument("--after", type=Path, required=True, help="registered corpus dir")
    parser.add_argument("--output", type=Path, required=True, help="output PNG path")
    parser.add_argument(
        "--downsample",
        type=int,
        default=2,
        help="reduce output by this factor (default 2; use 1 for full res)",
    )
    args = parser.parse_args()

    # Use the registered dir's first page to fix the shared canvas size.
    registered_pages = sorted(p for p in args.after.iterdir() if p.is_file())
    if not registered_pages:
        print(f"error: no pages in {args.after}", file=sys.stderr)
        return 1
    canvas_size = Image.open(registered_pages[0]).size

    print(f"canvas {canvas_size}", file=sys.stderr)
    print(f"stacking before: {args.before}", file=sys.stderr)
    before_stack = stack_dir(args.before, canvas_size)
    print(f"stacking after:  {args.after}", file=sys.stderr)
    after_stack = stack_dir(args.after, canvas_size)

    composite = side_by_side(before_stack, after_stack)
    if args.downsample > 1:
        composite = composite.reduce(args.downsample)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    composite.save(args.output, optimize=True)
    print(f"wrote {args.output} ({composite.size[0]}x{composite.size[1]})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
