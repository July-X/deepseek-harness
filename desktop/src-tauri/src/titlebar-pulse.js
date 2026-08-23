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
 * - colour: an emerald-400 sweep instead of the whale-eye red, so the
 *   user can read at a glance which surface the chrome belongs to
 *   (workbench = green; management panel = red whale eye).
 * - tempo: 6.912s period — 10% faster than the shell panel's 7.68s —
 *   so the workbench pulse reads as a slightly more alert presence.
 *
 * Loading order between this script and the workbench's own base.css
 * is not guaranteed, so every rule carries `!important` — the wrapper
 * script also appends the style node on `DOMContentLoaded` (or
 * immediately, if the document has already finished parsing) so the
 * injected sheet lives at the end of `<head>` and therefore wins
 * specificity ties even without `!important`. Belt and braces.
 */
(function () {
  var css = [
    /* Hide the kernel's static brand band if it ever ships one. */
    "body::before { content: none !important; display: none !important; }",
    /* First sweep — left to right across the chrome row. */
    "body::after {",
    "  content: '' !important;",
    "  position: fixed !important;",
    "  top: 0 !important;",
    "  left: 0 !important;",
    "  height: 3px !important;",
    "  width: 38% !important;",
    "  z-index: 1000 !important;",
    "  pointer-events: none !important;",
    "  background: linear-gradient(90deg, transparent 0%, rgba(110, 231, 183, 0.55) 30%, #34d399 50%, rgba(110, 231, 183, 0.55) 70%, transparent 100%) !important;",
    "  filter: blur(0.4px) !important;",
    "  border-radius: 999px !important;",
    "  animation: dsh-workbench-titlebar-pulse-sweep 6.912s cubic-bezier(0.4, 0, 0.2, 1) infinite !important;",
    "  box-shadow: 0 0 8px rgba(52, 211, 153, 0.45) !important;",
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
    "  width: 38% !important;",
    "  z-index: 1000 !important;",
    "  pointer-events: none !important;",
    "  background: linear-gradient(90deg, transparent 0%, rgba(110, 231, 183, 0.55) 30%, #34d399 50%, rgba(110, 231, 183, 0.55) 70%, transparent 100%) !important;",
    "  filter: blur(0.4px) !important;",
    "  border-radius: 999px !important;",
    "  animation: dsh-workbench-titlebar-pulse-sweep 6.912s cubic-bezier(0.4, 0, 0.2, 1) infinite !important;",
    "  animation-delay: 3.456s !important;",
    "  box-shadow: 0 0 8px rgba(52, 211, 153, 0.45) !important;",
    "}",
    "@keyframes dsh-workbench-titlebar-pulse-sweep {",
    "  0% { transform: translateX(-120%); opacity: 0; }",
    "  15% { opacity: 1; }",
    "  85% { opacity: 1; }",
    "  100% { transform: translateX(360%); opacity: 0; }",
    "}",
  ].join("\n");

  function inject() {
    if (document.getElementById("dsh-workbench-titlebar-pulse")) return;
    var style = document.createElement("style");
    style.id = "dsh-workbench-titlebar-pulse";
    style.textContent = css;
    document.head.appendChild(style);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", inject);
  } else {
    inject();
  }
})();