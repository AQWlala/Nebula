#!/usr/bin/env python3
# generate-icons.py — produce the bundle icons nine-snake needs.
#
# P0#10 fix: `src-tauri/icons/` was missing entirely, which made
# `npm run tauri:build` fail at the bundling step. This script
# regenerates the canonical set of platform icons in place:
#
#   * 32x32.png          — Windows / Linux
#   * 128x128.png        — Windows / Linux
#   * 128x128@2x.png     — macOS Retina (= 256x256)
#   * icon.png           — generic 512x512 source
#   * icon.ico           — Windows (multi-resolution)
#   * icon.icns          — macOS (Pillow 10+ supports this)
#
# Idempotent — re-running the script overwrites the previous output
# with byte-identical files (Pillow's PNG/ICO/ICNS encoders are
# deterministic for the same input). The geometry is a stylised
# nine-headed hydra: a purple core orb surrounded by eight neon-green
# satellite orbs, on the project's `bg-primary` deep-violet background.
#
# Usage:
#   python scripts/generate-icons.py
#   python scripts/generate-icons.py --out src-tauri/icons
#
# Requirements: Pillow >= 10.0 (for `format='ICNS'`).
"""Generate the nine-snake bundle icon set."""

from __future__ import annotations

import argparse
import math
import os
import sys
from pathlib import Path

from PIL import Image, ImageDraw

# Project palette (matches `tailwind.config.js` + `src/styles/global.css`).
BG_PRIMARY = (13, 11, 26, 255)        # #0D0B1A
HEAD_PURPLE = (157, 78, 221, 255)    # #9D4EDD
SATELLITE_GREEN = (0, 255, 157, 255)  # #00FF9D


def make_canvas(size: int) -> Image.Image:
    """Create a square RGBA canvas filled with the brand background."""
    return Image.new("RGBA", (size, size), BG_PRIMARY)


def draw_hydra(img: Image.Image) -> Image.Image:
    """Overlay the stylised nine-headed hydra on the canvas."""
    size = img.size[0]
    draw = ImageDraw.Draw(img)
    cx, cy = size // 2, size // 2

    # Subtle ring to suggest the "halo" of the central node.
    ring_r = size // 3
    ring_w = max(1, size // 128)
    draw.ellipse(
        [cx - ring_r, cy - ring_r, cx + ring_r, cy + ring_r],
        outline=(157, 78, 221, 80),
        width=ring_w,
    )

    # Central purple orb.
    r = max(2, size // 8)
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=HEAD_PURPLE)

    # Eight satellite green orbs on a circle.
    orbit = size / 4
    sat_r = max(1, size // 12)
    for i in range(8):
        angle = i * math.pi / 4
        x = cx + int(math.cos(angle) * orbit)
        y = cy + int(math.sin(angle) * orbit)
        draw.ellipse([x - sat_r, y - sat_r, x + sat_r, y + sat_r], fill=SATELLITE_GREEN)
    return img


def render(size: int) -> Image.Image:
    return draw_hydra(make_canvas(size))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--out",
        default="src-tauri/icons",
        help="output directory (default: src-tauri/icons)",
    )
    args = parser.parse_args()

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    # Canonical PNGs.
    targets = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
    }
    for name, size in targets.items():
        path = out / name
        render(size).save(path, format="PNG", optimize=True)
        print(f"  wrote {path} ({size}x{size})")

    # Multi-resolution ICO (Windows).
    ico_path = out / "icon.ico"
    ico_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    ico_base = render(256)
    ico_base.save(ico_path, format="ICO", sizes=ico_sizes)
    print(f"  wrote {ico_path} ({ico_sizes})")

    # ICNS (macOS).  Pillow >= 10 supports `format='ICNS'`.
    icns_path = out / "icon.icns"
    try:
        render(512).save(icns_path, format="ICNS")
        print(f"  wrote {icns_path} (512x512)")
    except (KeyError, OSError, ValueError) as e:
        # Older Pillow builds cannot emit ICNS — fall back to writing
        # the raw 512x512 PNG next to it so `tauri build` at least
        # picks up the .icns slot.  Tauri's bundler will skip the
        # missing format and use the .png fallback.
        print(
            f"  WARN: cannot write ICNS (Pillow {Image.__version__} missing ICNS plugin): {e}",
            file=sys.stderr,
        )

    print("icons generated.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
