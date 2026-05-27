"""Read the PDF MediaBox and print the right --paper / --paper-mm for register.

**You cannot determine a book's trim size from the PBM the scanner pipeline
produces.** `pdftoppm` renders the PDF page at a user-chosen DPI; the
resulting image's pixel count is `pdf_page_mm * dpi / 25.4` — i.e. the
PDF's stored page dimensions scaled, not the scanner's original resolution.

So to align "to the actual book trim" you have to read the PDF MediaBox.
This script shells out to `pdfinfo` (poppler), pulls the page dimensions
in points (1 pt = 25.4 / 72 mm), converts to mm, and prints the nearest
named standard from `register_core::Paper` — or the exact mm value for
`--paper-mm` when the trim is non-standard (the common case for Japanese
publishers, who routinely cut a few mm under nominal A6 / 四六判 / etc.).

Usage:
    python tools/pdf_paper.py file.pdf [file2.pdf ...]

Output (one line per PDF):

    file.pdf  PDF page 123.20x187.21 mm  matches 'shiroku' (127x188 mm, off by 3.8 / 0.8 mm)
              suggested CLI: --paper shiroku   OR --paper-mm 123.20x187.21
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

# Named standards — must stay in sync with `register_core::Paper::dimensions_mm()`.
STANDARDS: list[tuple[str, float, float]] = [
    ("a4", 210.0, 297.0),
    ("a5", 148.0, 210.0),
    ("a6", 105.0, 148.0),
    ("b5", 182.0, 257.0),
    ("b6", 128.0, 182.0),
    ("shinsho", 103.0, 182.0),
    ("shiroku", 127.0, 188.0),
    ("iso-b5", 176.0, 250.0),
]

PT_PER_MM = 72.0 / 25.4

PAGE_SIZE_PATTERN = re.compile(r"Page size:\s+([\d.]+)\s+x\s+([\d.]+)\s+pts")


def page_size_mm(pdf: Path) -> tuple[float, float] | None:
    """Return the PDF's first-page (width, height) in mm, or None on failure."""
    try:
        out = subprocess.run(
            ["pdfinfo", str(pdf)],
            check=True,
            capture_output=True,
            text=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError) as e:
        print(f"  pdfinfo failed: {e}", file=sys.stderr)
        return None
    match = PAGE_SIZE_PATTERN.search(out.stdout)
    if not match:
        print(f"  no Page size line in pdfinfo output for {pdf}", file=sys.stderr)
        return None
    w_pt, h_pt = float(match.group(1)), float(match.group(2))
    # 1 pt = 25.4 / 72 mm
    return (w_pt / PT_PER_MM, h_pt / PT_PER_MM)


def nearest_standard(width_mm: float, height_mm: float) -> tuple[str, float, float, float]:
    """Find the closest named standard.

    Returns `(name, std_w, std_h, abs_distance_mm)` — sum of the absolute
    width + height deltas, as a single scalar so we can pick the most
    sensible match.
    """
    # Normalize to portrait so the comparison doesn't trip on landscape pages.
    w, h = sorted((width_mm, height_mm))
    best = ("", 0.0, 0.0, float("inf"))
    for name, sw, sh in STANDARDS:
        distance = abs(w - sw) + abs(h - sh)
        if distance < best[3]:
            best = (name, sw, sh, distance)
    return best


def main() -> int:
    """CLI entry point."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument("pdfs", type=Path, nargs="+", help="PDF files to inspect")
    parser.add_argument(
        "--threshold-mm",
        type=float,
        default=2.0,
        help=(
            "Maximum per-dimension distance to a named standard before we "
            "stop recommending it (default 2.0 mm; Japanese publishers "
            "routinely cut up to ~4 mm under nominal so the named match is "
            "still printed as a 'close-but-not-exact' suggestion)."
        ),
    )
    args = parser.parse_args()

    rc = 0
    for pdf in args.pdfs:
        print(f"=== {pdf} ===")
        size = page_size_mm(pdf)
        if size is None:
            rc = 1
            continue
        w_mm, h_mm = size
        name, sw, sh, _ = nearest_standard(w_mm, h_mm)
        dw, dh = abs(w_mm - sw), abs(h_mm - sh)
        nominal_match = dw < args.threshold_mm and dh < args.threshold_mm

        print(f"  PDF MediaBox: {w_mm:.2f} x {h_mm:.2f} mm")
        print(f"  Nearest:      {name} ({sw:.0f} x {sh:.0f} mm, off by {dw:.2f} / {dh:.2f} mm)")
        if nominal_match:
            print(f"  → use:        --paper {name}")
        else:
            print(f"  → use:        --paper-mm {w_mm:.2f}x{h_mm:.2f}")
            print(f"     (closer to actual trim; --paper {name} would add visible margins)")
        print()
    return rc


if __name__ == "__main__":
    sys.exit(main())
