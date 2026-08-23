#!/usr/bin/env bash
# Regenerate every icon asset in the repo from the two SVG masters:
#   assets/whale-icon.svg        → sizes ≥ 128 (full eye detail: glow, star sparkle, rays)
#   assets/whale-icon-small.svg  → sizes ≤ 64 and favicons (exaggerated eye; detail would be subpixel)
#
# Outputs:
#   src-tauri/icons/{32x32,128x128,128x128@2x,icon}.png, icon.ico, icon.icns
#   assets/whale-icon-512.png
#   ui/whale-icon.png           (rendered at 128 from the SMALL master: the panel shows it at 60 CSS px)
#   ../website/public/favicon.svg   (brand-blue whale)
#   ../apps/web/public/favicon.svg  (black whale, white under dark color scheme; pwa-manifest.e2e.ts
#                                    pins the media query and fill="#000")
#
# Eye rays are <polygon>, never <path>, so the apps/web dark-scheme `path { fill: #fff }`
# rule cannot bleach them. Requires rsvg-convert, ImageMagick (magick) and macOS iconutil.
#
# Small sizes (≤64) render the SVG at a 16× supersampled canvas then downsample
# with LanczosSharp. rsvg-convert (cairo) directly downsampling radialGradient
# at 16/24/32/48/64 collapses sub-pixel detail (white highlight dot, spark rays)
# into a single pink blob. Supersampling preserves those edges, LanczosSharp
# gives crisp icon-style downsampling without the soft blur of plain Lanczos.
set -euo pipefail
cd "$(dirname "$0")/.."

ICONS=src-tauri/icons
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Large sizes: rsvg-convert directly — no downsampling needed, cairo handles
# them well and the result is the source of truth for retina/master assets.
for s in 128 256 512 1024; do
  rsvg-convert -w "$s" -h "$s" assets/whale-icon.svg -o "$TMP/master-$s.png"
done

# Small sizes: render at 16× on a clean canvas, then LanczosSharp downsample.
# 16× is chosen because the SVG viewBox is 50 and 1024/16 = 64 stays well below
# rsvg-convert's numeric limits while keeping supersample overhead trivial
# (~1 MP per frame × 5 frames).
SUPER=1024
rsvg-convert -w "$SUPER" -h "$SUPER" assets/whale-icon-small.svg -o "$TMP/small-super.png"
for s in 16 24 32 48 64; do
  magick "$TMP/small-super.png" -filter LanczosSharp -resize "${s}x${s}" \
    -define png:color-type=6 "$TMP/small-$s.png"
done

# ui/whale-icon.png stays at 128 from the SMALL master: the panel renders it
# at 60 CSS px so small-master geometry is correct, but we still supersample
# to keep the white highlight dot and spark rays sharp instead of cairo-blurred.
magick "$TMP/small-super.png" -filter LanczosSharp -resize 128x128 \
  -define png:color-type=6 "$TMP/small-128.png"

# Desktop bitmaps.
cp "$TMP/small-32.png" "$ICONS/32x32.png"
cp "$TMP/master-128.png" "$ICONS/128x128.png"
cp "$TMP/master-256.png" "$ICONS/128x128@2x.png"
cp "$TMP/master-512.png" "$ICONS/icon.png"
cp "$TMP/master-512.png" assets/whale-icon-512.png
cp "$TMP/small-128.png" ui/whale-icon.png

# Windows .ico: per-size frames, small variant below 128.
magick \
  "$TMP/small-32.png" "$TMP/small-16.png" "$TMP/small-24.png" \
  "$TMP/small-48.png" "$TMP/small-64.png" "$TMP/master-256.png" \
  "$ICONS/icon.ico"

# macOS .icns via an iconset; retina @2x frames reuse the next size up.
ICONSET="$TMP/whale.iconset"
mkdir "$ICONSET"
cp "$TMP/small-16.png" "$ICONSET/icon_16x16.png"
cp "$TMP/small-32.png" "$ICONSET/icon_16x16@2x.png"
cp "$TMP/small-32.png" "$ICONSET/icon_32x32.png"
cp "$TMP/small-64.png" "$ICONSET/icon_32x32@2x.png"
cp "$TMP/master-128.png" "$ICONSET/icon_128x128.png"
cp "$TMP/master-256.png" "$ICONSET/icon_128x128@2x.png"
cp "$TMP/master-256.png" "$ICONSET/icon_256x256.png"
cp "$TMP/master-512.png" "$ICONSET/icon_256x256@2x.png"
cp "$TMP/master-512.png" "$ICONSET/icon_512x512.png"
cp "$TMP/master-1024.png" "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o "$ICONS/icon.icns"
