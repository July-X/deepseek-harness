/**
 * Initialization script injected by `commands::open_harness` into the
 * dsh web workbench webview. It overrides the kernel's
 * `packages/client/web/src/base.css` titlebar styling so the desktop
 * shell owns the chrome-row pulse regardless of which kernel version
 * the workbench is running against.
 *
 * Why inject from the shell rather than wait for a kernel-side fix:
 * the kernel is a published npm artefact (`@deepseek-ai/dsh@<ver>`),
 * and a chrome-row decoration is shell chrome, not workbench content.
 * Letting the shell override keeps the workbench content surface
 * stable while the chrome evolves with the shell.
 *
 * The workbench intentionally diverges from the desktop shell panel on
 * two axes:
 *
 * - colour: the Gitea brand green (#609926, rgb 96,152,38) — pure green,
 *   warmer than Tailwind's emerald-400 — so the user can read at a
 *   glance which surface the chrome belongs to (workbench = Gitea
 *   green; management panel = red whale eye).
 * - tempo: 6.01s period — ~22% faster than the shell panel's 7.68s —
 *   so the workbench pulse reads as a slightly more alert presence.
 *
 * Why the keyframe endpoints are computed from `getBoundingClientRect()`
 * and re-computed on `resize`: in WKWebView, `documentElement.clientWidth`
 * returns physical pixels (e.g. 2560 on a 2x retina display), while CSS
 * layout and `transform` operate in CSS pixels (e.g. 1280). Hard-coding
 * endpoints from `clientWidth` therefore produces a transform range that
 * is 2x the actual visible width — the band reaches the right edge of
 * the compositing layer halfway through the animation and is clipped,
 * then the cycle resets and the band snaps back to the left edge. The
 * visual symptom is "the band vanishes mid-screen and reappears at the
 * left". `getBoundingClientRect().width` returns CSS pixels, matching
 * the coordinate system that `transform` and `width` use. A `resize`
 * listener re-computes the endpoints whenever the window is resized or
 * the display scale changes, so the band always travels exactly one
 * viewport-width plus one band-width.
 *
 * Loading order between this script and the workbench's own base.css
 * is not guaranteed, so every rule carries `!important` — the wrapper
 * script also appends the style node on `DOMContentLoaded` (or
 * immediately, if the document has already finished parsing) so the
 * injected sheet lives at the end of `<head>` and therefore wins
 * specificity ties even without `!important`. Belt and braces.
 */
(function () {
  var STYLE_ID = "dsh-workbench-titlebar-pulse";

  function cssViewportWidth() {
    return document.documentElement.getBoundingClientRect().width
      || document.documentElement.clientWidth
      || window.innerWidth;
  }

  function buildCss() {
    var viewportPx = cssViewportWidth();
    // Width: 18.24% of the layout viewport, expressed in CSS pixels.
    var bandPx = viewportPx * 0.1824;
    // Keyframe endpoints in CSS pixels, not vw:
    //   0%   → band leading edge one band-width past the left edge
    //   100% → band trailing edge one band-width past the right edge
    var startPx = -bandPx;
    var endPx = viewportPx + bandPx;
    return [
      /* Hide the kernel's static brand band if it ever ships one. */
      "body::before { content: none !important; display: none !important; }",
      /* First sweep — left to right across the chrome row. */
      "body::after {",
      "  content: '' !important;",
      "  position: fixed !important;",
      "  top: 0 !important;",
      "  left: 0 !important;",
      "  height: 3px !important;",
      "  width: " + bandPx + "px !important;",
      "  z-index: 1000 !important;",
      "  pointer-events: none !important;",
      "  background: linear-gradient(90deg, transparent 0%, rgba(96, 152, 38, 0.55) 15%, #609926 50%, rgba(96, 152, 38, 0.55) 85%, transparent 100%) !important;",
      "  filter: blur(0.4px) !important;",
      "  border-radius: 999px !important;",
      "  animation: dsh-workbench-titlebar-pulse-sweep 6.01s linear infinite !important;",
      "  box-shadow: 0 0 8px rgba(96, 152, 38, 0.45) !important;",
      "}",
      /* Second sweep — same width, half-cycle offset so the eye reads a
         continuous scan rather than a single dash crossing then dead
         space. BootPage.installTitlebarPulse() injects the DOM node on
         boot; this rule simply matches it. */
      "body > [data-titlebar-pulse='2'] {",
      "  position: fixed !important;",
      "  top: 0 !important;",
      "  left: 0 !important;",
      "  height: 3px !important;",
      "  width: " + bandPx + "px !important;",
      "  z-index: 1000 !important;",
      "  pointer-events: none !important;",
      "  background: linear-gradient(90deg, transparent 0%, rgba(96, 152, 38, 0.55) 15%, #609926 50%, rgba(96, 152, 38, 0.55) 85%, transparent 100%) !important;",
      "  filter: blur(0.4px) !important;",
      "  border-radius: 999px !important;",
      "  animation: dsh-workbench-titlebar-pulse-sweep 6.01s linear infinite !important;",
      "  animation-delay: 3.005s !important;",
      "  box-shadow: 0 0 8px rgba(96, 152, 38, 0.45) !important;",
      "}",
      "@keyframes dsh-workbench-titlebar-pulse-sweep {",
      "  0% { transform: translateX(" + startPx + "px); opacity: 1; }",
      "  100% { transform: translateX(" + endPx + "px); opacity: 1; }",
      "}",
    ].join("\n");
  }

  function inject() {
    var existing = document.getElementById(STYLE_ID);
    if (existing) {
      // Recompute on resize: replace the sheet content so the keyframes
      // track the current CSS viewport width.
      existing.textContent = buildCss();
      return;
    }
    var style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = buildCss();
    document.head.appendChild(style);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", inject);
  } else {
    inject();
  }

  // Recompute the keyframes whenever the viewport changes size or scale.
  // WKWebView fires `resize` on zoom, window resize, and display scale
  // changes, so this single listener covers every case where the CSS
  // pixel width of the viewport shifts.
  var resizeTimer;
  window.addEventListener("resize", function () {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(inject, 100);
  });
})();
