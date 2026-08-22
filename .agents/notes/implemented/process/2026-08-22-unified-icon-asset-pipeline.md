# Agent Note: Unified icon asset pipeline

Status: implemented

English | [中文](2026-08-22-unified-icon-asset-pipeline.zh.md)

## Problem

The whale icon existed as hand-copied files that drifted per surface: `desktop/src-tauri/icons/` came from a single 512px PNG through `tauri icon`, `website/public/favicon.svg` was a brand-blue whale without the red eye, `apps/web/public/favicon.svg` was a dark-scheme-adaptive whale without the red eye, and `desktop/ui/whale-icon.png` was rendered by hand. The red-eye redesign made the drift visible, and the detailed eye (glow, star sparkle, rays at r=0.62 in a 50-unit viewBox) is subpixel at 16–64px, so a single master physically cannot serve every size.

## Decision

Two SVG masters in `desktop/assets/` are the only hand-edited icon sources: `whale-icon.svg` (full eye detail, for ≥128px) and `whale-icon-small.svg` (exaggerated eye — larger glow and core, bolder rays, one bold glint instead of the subpixel star — for ≤64px and favicon projections). `desktop/scripts/build-icons.sh` regenerates every derivative in one run: `src-tauri/icons/` (per-size frames composed into `icon.ico` via ImageMagick and `icon.icns` via an iconset + `iconutil`, each frame rendered from the appropriate master rather than downscaled from one bitmap), `assets/whale-icon-512.png`, `ui/whale-icon.png` (from the small master — the panel sidebar shows it at 60 CSS px, so the detailed eye would be subpixel), and the two SVG favicons as text projections of the small master — `website` gets a brand-blue (`#4D6BFE`) whale body, `apps/web` gets `fill="#000"` plus the `prefers-color-scheme: dark` style block that `apps/web/tests/pwa-manifest.e2e.ts` pins.

Eye rays are `<polygon>`, never `<path>`, in both masters: the apps/web dark-scheme rule selects `path`, and a path-based ray would be bleached white along with the whale body.

`tauri icon` is no longer part of the flow; it regenerates every frame from one master and would silently revert the small-size frames to the detailed design.

## Alternatives considered

**One master, `tauri icon` for everything.** The previous flow, and the least tooling. It cannot express size-dependent design: at 16–64px the detailed eye is invisible, so the brand feature disappears exactly where icons are most often seen.

**Per-surface hand edits.** Each surface keeps its own file and an editor syncs them by hand. This is what produced the drift being fixed: three files, three silhouettes, two eye treatments, no procedure connecting them.

**A third favicon master.** Favicons are small icons; the small master already carries exactly the simplification they need. A separate file would only re-open the drift this pipeline closes.

## Consequences

A design change touches at most two SVGs plus one script run, and every png/ico/icns/favicon in the repository follows. The script requires rsvg-convert, ImageMagick, and macOS `iconutil`, so icon regeneration is a macOS step; CI does not consume it. `website/public/wordmark.svg` is unaffected — it contains no whale glyph. The `apps/web` favicon keeps its dark-scheme behavior, and the website favicon keeps its brand-blue body: what unified is the silhouette and the red-eye design, not the per-medium color treatment.
