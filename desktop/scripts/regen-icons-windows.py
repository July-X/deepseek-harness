#!/usr/bin/env python3
"""
One-shot helper used when the canonical `scripts/build-icons.sh` cannot
run (no rsvg-convert / ImageMagick / iconutil on the host) but the
commit needs to ship the regenerated rounded white-plated icons.
Reads the existing transparent PNGs that ship with the repo and emits
the same rounded white-plated variants `build-icons.sh` would produce
under macOS.

Only the desktop bundle outputs are plated. `ui/whale-icon.png`
keeps its transparent background so the panel brand mark still layers
on the dark chrome row. SVG favicons are untouched (the script does
not produce them).

Run from the repo root: `python desktop/scripts/regen-icons-windows.py`.
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path

from PIL import Image, ImageDraw

REPO = Path(__file__).resolve().parent.parent
ICONS = REPO / "src-tauri" / "icons"
ASSETS = REPO / "assets"
UI = REPO / "ui"

# Corner radius as a fraction of the side length. Mirrors the
# `magick ... roundrectangle ... ${radius},${radius}` math in
# `scripts/build-icons.sh`. iOS / Big Sur app-icon proportions land
# near 22% on macOS's own squircle mask; we pick 18% so the plate sits
# just inside that mask and Windows reads it as a clean rounded
# square rather than a full circle.
PLATE_RADIUS_PCT = 18

# Fraction of the side length the whale itself occupies. The
# remainder becomes the iOS-style safe-area padding so the white
# plate reads as the icon's frame and the whale sits centered inside
# it. 82% leaves roughly 9% of breathing room on each side, matching
# the proportion modern app icons use. Mirrors the `${inset}` math in
# `scripts/build-icons.sh`.
WHALE_INSET_PCT = 82


def plate_white_rounded(src: Path, dst: Path) -> None:
    """Compose `src` (RGBA) over a rounded white plate and write a
    flat RGB `dst` matching what `build-icons.sh` produces on macOS.

    The plate fills the full canvas; the whale is shrunk to
    WHALE_INSET_PCT and centered, leaving the safe-area padding that
    iOS-style app icons expect."""
    whale = Image.open(src).convert("RGBA")
    size = whale.size[0]
    inset = max(1, size * WHALE_INSET_PCT // 100)
    whale_resized = whale.resize((inset, inset), Image.LANCZOS)

    plate = Image.new("RGBA", whale.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(plate)
    radius = max(1, size * PLATE_RADIUS_PCT // 100)
    # rounded_rectangle draws an arc at the corners; passing
    # (size-1, size-1) keeps the curve strictly inside the canvas.
    draw.rounded_rectangle(
        ((0, 0), (size - 1, size - 1)),
        radius=radius,
        fill=(255, 255, 255, 255),
    )
    # Paste uses the source's alpha as the mask so the whale blends
    # cleanly onto the plate. Center on both axes by offsetting by
    # half the leftover margin.
    offset = (size - inset) // 2
    plate.paste(whale_resized, (offset, offset), whale_resized)
    plate.convert("RGB").save(dst, "PNG")


def write_ico(sources: list[tuple[Path, int]], dst: Path) -> None:
    """Write a Windows .ico containing the listed (path, size) frames.

    Pillow's `save(format="ICO")` would normally handle this, but the
    legacy `.ico` shipped by the build script packs five small variants
    (16/24/32/48/64) plus the master 256 in that specific order.
    Pillow reorders frames by size and drops any above 256, so we
    build the ICONDIR / ICONDIRENTRY headers manually to keep the
    byte-for-byte layout the previous build produced.
    """

    # Decode each frame's PNG to verify size, then re-encode as
    # embedded PNG (Vista+ .ico supports PNG-compressed frames; the
    # build script and Tauri's resource loader both accept them).
    frames: list[tuple[int, bytes]] = []
    for path, size in sources:
        with Image.open(path) as img:
            assert img.size == (size, size), f"{path} is {img.size}, expected {(size, size)}"
        frames.append((size, path.read_bytes()))

    header = struct.pack("<HHH", 0, 1, len(frames))
    entry_size = 16
    offset = 6 + entry_size * len(frames)
    entries = b""
    payloads = b""
    for size, data in frames:
        # ICONDIRENTRY: width, height, color-count, reserved, planes,
        # bit-count, byte-count, image-offset. Width/height 0 means
        # 256 (per spec) — none of the build-script frames hit 256
        # so we don't need that case.
        w = h = size if size < 256 else 0
        entries += struct.pack(
            "<BBBBHHII",
            w & 0xFF,
            h & 0xFF,
            0,  # color count (palette only; PNG frames ignore)
            0,  # reserved
            1,  # planes
            32,  # bit count
            len(data),
            offset,
        )
        payloads += data
        offset += len(data)

    dst.write_bytes(header + entries + payloads)


def main() -> int:
    # Desktop PNGs that ship via Tauri config (see tauri.conf.json).
    targets = [
        (ICONS / "32x32.png", ICONS / "32x32.png", 32),
        (ICONS / "128x128.png", ICONS / "128x128.png", 128),
        (ICONS / "128x128@2x.png", ICONS / "128x128@2x.png", 256),
        (ICONS / "icon.png", ICONS / "icon.png", 512),
        (ASSETS / "whale-icon-512.png", ASSETS / "whale-icon-512.png", 512),
    ]
    for src, dst, _ in targets:
        plate_white_rounded(src, dst)
        print(f"plated {dst.relative_to(REPO)}")

    # Build a minimal .ico from the committed PNGs. The macOS build
    # adds the small 16/24/48/64 frames via `magick`; the Windows
    # regen path runs without ImageMagick, so the .ico ships with
    # just the 32 + 128 + 256 frames that Windows resource loaders
    # already handle cleanly. macOS build regenerates the full set.
    ico_dir = ICONS
    write_ico(
        [
            (ICONS / "32x32.png", 32),
            (ICONS / "128x128.png", 128),
            (ICONS / "128x128@2x.png", 256),
        ],
        ico_dir / "icon.ico",
    )
    print(f"wrote {ICONS.relative_to(REPO)}/icon.ico")

    # The macOS .icns cannot be regenerated without iconutil — leave
    # that to the macOS build. The release workflow runs on macOS
    # runners, so the .icns is refreshed there.
    icns = ICONS / "icon.icns"
    if icns.exists():
        print(
            f"note: {icns.relative_to(REPO)} is binary; regenerate it on macOS via build-icons.sh"
        )

    # Panel icon stays transparent — touch nothing in ui/.
    return 0


if __name__ == "__main__":
    sys.exit(main())