#!/usr/bin/env python3
"""Draws the desktop app icons from the Renderd icon mark.

The mark is a handful of rectangles on a 120 unit grid (see
assets/brand/icon.svg), so it is drawn directly instead of rasterising the
SVG. Needs Pillow, and iconutil for the macOS .icns file.

    python3 tools/desktop-icons/generate.py
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "crates" / "renderd-desktop" / "icons"

VOID = (13, 13, 13, 255)
EDGE = (38, 38, 38, 255)
INK = (232, 244, 253, 255)
ACCENT = (0, 229, 255, 255)

# (x, y, width, height, colour) on the 120 unit grid of icon.svg.
MARK = [
    (28, 24, 8, 72, INK),
    (28, 24, 44, 8, INK),
    (28, 56, 44, 8, INK),
    (72, 24, 8, 40, INK),
    (56, 64, 8, 8, INK),
    (64, 72, 8, 8, INK),
    (72, 80, 8, 8, INK),
    (80, 88, 8, 8, INK),
    (80, 24, 8, 8, ACCENT),
]


def draw(size: int, inset: float) -> Image.Image:
    """Returns the icon at `size` pixels with the tile inset by `inset` of it."""
    scale = 4
    canvas = size * scale
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    pen = ImageDraw.Draw(image)

    margin = canvas * inset
    tile = canvas - 2 * margin
    radius = tile * 0.225
    pen.rounded_rectangle(
        (margin, margin, canvas - margin, canvas - margin),
        radius=radius,
        fill=VOID,
        outline=EDGE,
        width=max(1, round(tile / 256)),
    )

    unit = tile / 120
    for x, y, width, height, colour in MARK:
        left = margin + x * unit
        top = margin + y * unit
        pen.rectangle(
            (round(left), round(top), round(left + width * unit) - 1, round(top + height * unit) - 1),
            fill=colour,
        )
    return image.resize((size, size), Image.LANCZOS)


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)

    # macOS icons sit inside a margin; Windows and Linux ones fill the square.
    draw(32, 0.0).save(OUT / "32x32.png")
    draw(128, 0.0).save(OUT / "128x128.png")
    draw(256, 0.0).save(OUT / "128x128@2x.png")
    draw(512, 0.0).save(OUT / "icon.png")

    draw(256, 0.0).save(
        OUT / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (256, 256)],
    )

    iconutil = shutil.which("iconutil")
    if iconutil is None:
        print("iconutil not found, skipping icon.icns", file=sys.stderr)
        return 0
    with tempfile.TemporaryDirectory() as scratch:
        iconset = Path(scratch) / "icon.iconset"
        iconset.mkdir()
        for points in (16, 32, 128, 256, 512):
            draw(points, 0.1).save(iconset / f"icon_{points}x{points}.png")
            draw(points * 2, 0.1).save(iconset / f"icon_{points}x{points}@2x.png")
        subprocess.run(
            [iconutil, "-c", "icns", str(iconset), "-o", str(OUT / "icon.icns")],
            check=True,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
