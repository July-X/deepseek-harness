# Agent Note: Workbench titlebar pulse keyframes — move the fade off-screen without shortening the sweep

Status: implemented

English | [中文](2026-08-23-workbench-titlebar-pulse-dead-time.zh.md)

## Problem

The desktop shell owns the dsh web workbench chrome by injecting a `<style>` element into the workbench webview at boot (`desktop/src-tauri/src/titlebar-pulse.js`, loaded via `WebviewWindowBuilder::initialization_script()` per the chrome-row pulse the shell installs for both its own panel and the kernel's web workbench). The two sweeps used:

```css
@keyframes dsh-workbench-titlebar-pulse-sweep {
  0%   { transform: translateX(-120%); opacity: 0; }
  15%  { opacity: 1; }
  85%  { opacity: 1; }
  100% { transform: translateX(360%); opacity: 0; }
}
```

with `animation: ... 6.912s cubic-bezier(0.4, 0, 0.2, 1) infinite;` and a half-period-offset second sweep on the same band.

A 38% wide band on a ~480px chrome row means the band fully enters the visible row around `translateX(-100%)` and fully exits around `translateX(+162%)`. The 0% keyframe parked the band at `translateX(-120%)` (already a fifth of a band-width past the left edge) and the 100% keyframe at `translateX(360%)` (well past the right edge). The opacity ramp 0 → 1 over the first 15% and 1 → 0 over the last 15% therefore ran partly *while the band was inside the chrome row*, and `cubic-bezier(0.4, 0, 0.2, 1)` decelerates near both ends — the user reads the band as "sweeps once, then nothing happens for several seconds before it sweeps again".

Three iterations of this fix landed in sequence:

1. **First attempt**: shortened the sweep to `translateX(-100%) → 120%` and tightened the opacity ramps. Rejected by user review: visible sweep slowed, and `translateX(120%)` placed the fade-out while the trailing half was still on-screen — the band appeared to "vanish mid-flight".
2. **Second attempt**: kept the original 480% travel arc (`-120% → 360%`) and moved the opacity ramps to 8% / 85% so the fade happens off-screen. Rejected by user review: `cubic-bezier(0.4, 0, 0.2, 1)` has its peak speed at t ≈ 0.30 and decelerates monotonically from there to t = 1.0. The 8% → 85% segment — the only segment where the band is on-screen — therefore starts accelerating, hits peak speed around t = 0.30, then decelerates for the rest of the visible transit, ending with the band suddenly slowing down at the right edge. Read as "the band speeds up, then suddenly slows down and disappears".
3. **Third (current) attempt**: same keyframes as the second, but `animation-timing-function: linear` instead of `cubic-bezier(0.4, 0, 0.2, 1)`. The band now travels at constant speed across the visible chrome row, with the opacity ramps still fully off-screen as before.

## Decision

Two changes together: rewrite the keyframes so the opacity ramp happens *while the band is outside the chrome row*, and switch the animation timing function from `cubic-bezier(0.4, 0, 0.2, 1)` to `linear` so the band travels at constant speed while it is on-screen.

```css
@keyframes dsh-workbench-titlebar-pulse-sweep {
  0%   { transform: translateX(-120%); opacity: 0; }
  8%   { transform: translateX(-100%); opacity: 1; }
  85%  { transform: translateX(170%);  opacity: 1; }
  100% { transform: translateX(360%); opacity: 0; }
}

.row::after,
body > [data-titlebar-pulse='2'] {
  animation: dsh-workbench-titlebar-pulse-sweep 6.912s linear infinite;
}
```

Net effect on a 6.912 s cycle, with the band travelling through a ~480 px-wide chrome row at 480 px / 480%:

- 0% (translateX -120%, opacity 0) → 8% (translateX -100%, opacity 1): the band sits one band-width past the left edge while fading in. The leading edge crosses the row's left boundary right at 8%, in lock-step with opacity reaching 1. Linear timing means opacity ramps at constant rate over these 0.55 s.
- 8% → 85% (translateX travels -100% → 170%, opacity 1): the band is fully opaque across the entire visible row, exiting at 85% with its trailing edge just past the right boundary. Linear timing gives a constant travel speed — no acceleration in the middle and no deceleration at the edge. This 5.32 s slice is the only segment where the band is on-screen.
- 85% → 100% (translateX travels 170% → 360%, opacity 1 → 0): the band keeps sliding off-screen to translateX(360%) while opacity ramps to 0 at constant rate. Because the band is already past the row at 85%, the fade-out is invisible.

The two staggered sweeps keep their half-period `animation-delay` (3.456 s), so the second band is mid-traverse while the first begins. With the fade-out moved fully off-screen and the visible transit at constant speed, the worst-case gap between "trailing edge of band A leaves the row" and "leading edge of band B enters the row" shrinks to about the half-period minus the band's on-screen transit time — under one second, with no perceptible pause and no perceptible speed change.

The desktop shell panel's own `titlebar-pulse-sweep` keyframes (`desktop/ui/styles.css`) keep the same shape as before — that panel renders at a different tempo (7.68 s) and is not the surface the user reported, so changing it would be out of scope for this fix. The two keyframes intentionally diverge; if the shell panel ever shows the same dead-time symptom, it gets the same treatment in its own change.

The reason `linear` is the right timing function here: the band's travel is already a constant-velocity `translateX` sweep, and the fade ramps are short, off-screen, and short enough that a non-linear opacity ramp does not read as motion — only a constant-speed translation reads as a "scan". `cubic-bezier(0.4, 0, 0.2, 1)` (Material "standard") peaks at t ≈ 0.30 and decelerates monotonically from there to t = 1.0, which would give a perceptible "speeds up, then slows down and disappears" arc across the visible transit — exactly the symptom the third iteration removes.

## Alternatives considered

**Shorten the sweep arc to `translateX(-100%) → 120%` and tighten the opacity ramps.** Rejected: it preserved the 6.912 s period, so the band travelled less than half the original distance in the same time — the visible sweep slowed. Worse, `translateX(120%)` is only one band-width past the right edge of the chrome row, so the trailing half of the band was still inside the row when opacity reached 0; the band looked like it vanished mid-flight. This was the first attempt at the fix; the user caught it within minutes.

**Keep the rewritten keyframes but leave `cubic-bezier(0.4, 0, 0.2, 1)` as the timing function.** Rejected: the curve peaks at t ≈ 0.30 and decelerates monotonically from there to t = 1.0. With the on-screen transit living entirely inside the 8% → 85% segment (t ≈ 0.08 to t ≈ 0.85), the band accelerates from 8% through the early on-screen phase, peaks near the chrome row's midpoint, then decelerates all the way to the right edge. The user reads this as "the band speeds up, then suddenly slows down and disappears" — which is exactly what the third iteration removed. This was the second attempt; the user caught it during review.

**Lengthen the visible portion of the existing keyframes (e.g. 5% → 95% instead of 15% → 85%) but keep the same `translateX` endpoints.** Rejected: the start and end `transform` values still put the band partly inside the row during the fade, so the band's leading or trailing edge would fade in/out while still visible. Same perceived dead time, just shorter.

**Remove the fade altogether (opacity always 1, hard cut at 0% and 100%).** Rejected: the band's gradient already fades at its own edges; a hard opacity cut makes the leading and trailing edges pop in and out instead of smoothly fading. The 8% / 15% opacity ramps preserve that soft edge while moving the dead time off-screen.

**Synchronize the kernel-side `packages/client/web/src/base.css` titlebar pulse instead of overriding from the shell.** Rejected because the shell owns chrome-row styling per the workbench UX path in `desktop/AGENTS.md` — letting the shell override keeps the workbench content surface stable across kernel upgrades, which is exactly why the override file exists with its `!important` belt-and-braces belt.

**Switch from CSS animation to a JS-driven `requestAnimationFrame` loop on the band.** Rejected because it adds a Rust ↔ JS injection surface, breaks HMR, and reproduces what `animation-iteration-count: infinite` already gives for free.

**Use `ease-in-out` or another symmetric easing instead of `cubic-bezier(0.4, 0, 0.2, 1)`.** Rejected for the same reason as the second bullet: any curve that decelerates near the end will make the band "ease into" the right edge while opacity is still at 1, which is the visible artefact we are removing. `linear` is the only standard timing function whose speed is constant across the whole cycle.

## Consequences

The injected script's `css` array still names one keyframe (`dsh-workbench-titlebar-pulse-sweep`); the second sweep (`body > [data-titlebar-pulse='2']`) reuses it via `animation-delay`. The `!important` belt-and-braces comment in `titlebar-pulse.js` remains accurate: the injected sheet wins the cascade regardless of what the kernel's own `base.css` ships, and the rule block as a whole moves as one unit when the kernel catches up.

If a future kernel version adds its own titlebar pulse keyframes, the shell override will continue to suppress them through the `body::before { content: none }` and `body::after` rules. The fix here does not change that contract — only the keyframe geometry inside the override.

The change is a single `desktop/src-tauri/src/titlebar-pulse.js` edit. No Rust code paths move, no kernel packages are touched, and the desktop shell panel's own pulse is left at its current tempo. Verification is manual: open `dsh web` from a `tauri dev` build of the shell, watch the workbench chrome for at least one full 6.912 s cycle, and confirm (a) the band sweeps continuously with no perceptible pause between the first band's exit and the second band's entry, (b) the band's travel speed is constant across the visible chrome row — no perceptible acceleration, deceleration, or "settle" at either edge.

The first attempt at this fix landed in the working tree but was caught during user review before the commit was authored; this note documents the corrected keyframes and the trap that the first attempt fell into, so a future agent does not repeat it.
