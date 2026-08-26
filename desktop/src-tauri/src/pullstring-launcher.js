/**
 * Initialization script injected into both `open_harness` (the dsh web
 * workbench webview) and `open_official_chat` (the DeepSeek official chat
 * webview). It renders a small pull-string lamp floating at the top-left
 * of the page; pulling (clicking) the string lights the bulb and brings
 * the shell's main management window to the foreground via the
 * `focus_main_shell` command.
 *
 * Why inject from the shell rather than ship it in the kernel: the
 * kernel is a published npm artefact (`@deepseek-ai/dsh@<ver>`), and a
 * shortcut back to the shell's management panel is shell chrome, not
 * page content. The DeepSeek official chat is a third-party origin we
 * cannot ship scripts into, so the same injection is the only path.
 *
 * Two surfaces, two anchors and palettes:
 *
 * - dsh web workbench: Gitea green cord (#609926) at left:212px, sized
 *   to land beside the sidebar-collapse button (the comment near the
 *   offset documents the geometry in detail).
 *
 * - DeepSeek official chat: DeepSeek blue cord (#4D6BFE) at left:12px,
 *   hugging the chat's own top-left chrome — the chat page has no
 *   sidebar, so the workbench's 212px offset would put the lamp in
 *   empty space.
 *
 * The widget talks back to the shell over `window.__TAURI__.core.invoke`
 * (tauri.conf.json sets `withGlobalTauri: true`), so it needs no
 * page-side endpoint. Because both webviews are remote origins
 * (`http://127.0.0.1:<port>` and `https://chat.deepseek.com`),
 * Tauri's ACL only lets this invoke through because
 * `permissions/app-commands.json` exposes the command and the matching
 * `capabilities/harness-remote.json` / `capabilities/official-chat-remote.json`
 * grant it to the respective window on its origin.
 */
(function () {
  // Tauri runs initialization scripts in every frame; the widget belongs to
  // the top-level page only — without this guard an iframe inside chat
  // would mount its own lamp and double-paint the icon on every nested
  // document. The workbench page is a single document so the guard is a
  // harmless no-op there.
  if (window.top !== window.self) {
    return;
  }

  // Capture the real Tauri bridge ONCE at module top, BEFORE the
  // `chat-fingerprint.js` initialization script (which loads after this
  // file in the official-chat chain) overwrites `window.__TAURI__` with
  // a neutered Proxy. The lamp still works because we keep this
  // captured reference alive for the whole lifetime of the page. In the
  // harness webview there is no `chat-fingerprint.js`, so this capture
  // sees the real bridge on every load. Hoisted to the very top of the
  // IIFE so `invokeFocusMainShell` (below) can read it via a name that
  // does not depend on the live `window.__TAURI__` global — the lamp
  // keeps raising the management window even after the fingerprint
  // script neutered the bridge.
  var __DSH_TAURI_REF__ = (typeof window.__TAURI__ !== "undefined") ? window.__TAURI__ : null;

  // Surface selection by hostname: the dsh web workbench is a 127.0.0.1
  // origin and so is anything else we might inject into; chat.deepseek.com
  // is the only place the DeepSeek brand palette and the 12px offset apply.
  var isOfficial = window.location.hostname === "chat.deepseek.com";
  var PALETTE = isOfficial
    ? {
        cord: "#4D6BFE",
        cordHover: "#7C92FF",
      }
    : {
        cord: "#609926",
        cordHover: "#7dbd45",
      };
  var LEFT_PX = isOfficial ? "12px" : "212px";

  var ROOT_ID = "dsh-shell-launcher";
  var STYLE_ID = "dsh-shell-launcher-style";
  /** How long the bulb stays lit after a pull, in milliseconds. */
  var LIGHT_MS = 1600;

  var SVG = [
    '<svg viewBox="0 0 24 66" width="24" height="66" aria-hidden="true">',
    /* The string, hanging from the top edge of the viewport. */
    '<line class="dsh-launcher-cord" x1="12" y1="0" x2="12" y2="38"/>',
    /* Screw base where the string meets the bulb. */
    '<rect class="dsh-launcher-base" x="8.5" y="37" width="7" height="7" rx="1.5"/>',
    /* Bulb glass. */
    '<circle class="dsh-launcher-bulb" cx="12" cy="54" r="9.5"/>',
    /* Filament, visible when lit. */
    '<path class="dsh-launcher-filament" d="M9 52 q1.5 3 3 0 q1.5 -3 3 0" fill="none"/>',
    "</svg>",
  ].join("");

  function buildCss() {
    return [
      "#" + ROOT_ID + " {",
      "  position: fixed;",
      "  top: 0;",
      /* Hang next to the chrome corner of the page: 212px on the dsh web
         workbench (right of the brand logo, beside the sidebar-collapse
         button), 12px on chat.deepseek.com (no sidebar, hug the page's
         own top-left). Plain window resizes leave both anchors alone —
         the workbench's sidebar is fixed-width and the chat has nothing
         to align against. */
      "  left: " + LEFT_PX + ";",
      "  z-index: 2147483647;",
      "  pointer-events: none;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn {",
      "  pointer-events: auto;",
      "  display: block;",
      "  margin: 0;",
      "  padding: 0 6px;",
      "  border: 0;",
      "  background: none;",
      "  cursor: pointer;",
      "  transform-origin: 50% 0;",
      "  animation: dsh-shell-launcher-sway 6.4s ease-in-out infinite;",
      "  -webkit-tap-highlight-color: transparent;",
      "}",
      "#" + ROOT_ID + " svg {",
      "  display: block;",
      "  overflow: visible;",
      "  transition: transform 0.18s cubic-bezier(0.34, 1.56, 0.64, 1);",
      "  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.45));",
      "}",
      /* Cord color matches the surface brand (workbench green, official
         chat blue) so the lamp reads as the same chrome surface as the
         titlebar pulse running across the top of the viewport. */
      "#" + ROOT_ID + " .dsh-launcher-cord {",
      "  stroke: " + PALETTE.cord + ";",
      "  stroke-width: 2;",
      "  stroke-linecap: round;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-base {",
      "  fill: var(--dsh-launcher-base-fill);",
      "}",
      /* Palette variables, defaulting to the translucent whites that read
         against the dark page palette; the light-mode override near the
         end of the sheet swaps them. */
      "#" + ROOT_ID + " {",
      "  --dsh-launcher-base-fill: rgba(255, 255, 255, 0.42);",
      "  --dsh-launcher-bulb-fill: rgba(255, 255, 255, 0.14);",
      "  --dsh-launcher-bulb-stroke: rgba(255, 255, 255, 0.55);",
      "  --dsh-launcher-bulb-hover-stroke: rgba(255, 255, 255, 0.85);",
      "  --dsh-launcher-filament-stroke: rgba(255, 255, 255, 0.35);",
      "  --dsh-launcher-lit-fill: #ffd45e;",
      "  --dsh-launcher-lit-stroke: #ffdf8a;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-bulb {",
      "  fill: var(--dsh-launcher-bulb-fill);",
      "  stroke: var(--dsh-launcher-bulb-stroke);",
      "  stroke-width: 1.4;",
      "  transition: fill 0.15s ease-out, stroke 0.15s ease-out;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-filament {",
      "  stroke: var(--dsh-launcher-filament-stroke);",
      "  stroke-width: 1.2;",
      "  stroke-linecap: round;",
      "  transition: stroke 0.15s ease-out;",
      "}",
      /* Hover: the cord brightens to a lighter shade of the surface brand
         color to read as grabbable. */
      "#" + ROOT_ID + " .dsh-launcher-btn:hover .dsh-launcher-cord {",
      "  stroke: " + PALETTE.cordHover + ";",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn:hover .dsh-launcher-bulb {",
      "  stroke: var(--dsh-launcher-bulb-hover-stroke);",
      "}",
      /* Pulled: cord + bulb travel down together, springing back on release. */
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-pulled svg {",
      "  transform: translateY(6px);",
      "}",
      /* Lit: warm glass, glowing filament, halo around the bulb. */
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-on .dsh-launcher-bulb {",
      "  fill: var(--dsh-launcher-lit-fill);",
      "  stroke: var(--dsh-launcher-lit-stroke);",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-on .dsh-launcher-filament {",
      "  stroke: #b45309;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-on svg {",
      "  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.45)) drop-shadow(0 0 10px rgba(255, 212, 94, 0.85));",
      "}",
      /* Invoke failed (e.g. IPC unavailable): the bulb flashes red instead
         of staying warm, so a broken pull is visible without devtools. */
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-err .dsh-launcher-bulb {",
      "  fill: #ef4444;",
      "  stroke: #f87171;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-err svg {",
      "  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.45)) drop-shadow(0 0 10px rgba(239, 68, 68, 0.85));",
      "}",
      /* Light mode (the workbench marks dark with body[data-ds-dark-theme],
         written by boot-theme.ts before plugin load and kept by
         ThemePresenter after): amber glass and darker linework keep the
         bulb legible on the white workbench. The chat page does not set
         the attribute, so the rule still applies via :not — and it
         happens to render fine on chat's white surfaces too. Only the
         palette variables change, so hover/lit precedence stays
         identical across themes. */
      "body:not([data-ds-dark-theme]) #" + ROOT_ID + " {",
      "  --dsh-launcher-base-fill: #78716c;",
      "  --dsh-launcher-bulb-fill: rgba(245, 158, 11, 0.24);",
      "  --dsh-launcher-bulb-stroke: #a16207;",
      "  --dsh-launcher-bulb-hover-stroke: #854d0e;",
      "  --dsh-launcher-filament-stroke: #92400e;",
      "  --dsh-launcher-lit-fill: #f59e0b;",
      "  --dsh-launcher-lit-stroke: #92400e;",
      "}",
      "@keyframes dsh-shell-launcher-sway {",
      "  0%, 100% { transform: rotate(1.6deg); }",
      "  50% { transform: rotate(-1.6deg); }",
      "}",
    ].join("\n");
  }

  /**
   * Ask the shell to raise its management window next to the click.
   * `point` is the click's screen position (MouseEvent.screenX/Y, CSS
   * pixels); the shell moves the panel near it so the user does not
   * have to hunt for the window on another monitor. `onError` fires
   * when the Tauri IPC is unavailable or the command rejects, so the
   * caller can swap the warm glow for the red error flash.
   *
   * Uses the hoisted `__DSH_TAURI_REF__` rather than reading the live
   * `window.__TAURI__` — after `chat-fingerprint.js` runs (official-chat
   * chain only) the global has been replaced with a neutered Proxy that
   * rejects every `invoke`. The closure-captured reference still points
   * at the real bridge installed by `withGlobalTauri: true`, so the
   * lamp continues to raise the management window.
   */
  function invokeFocusMainShell(point, onError) {
    try {
      var tauri = __DSH_TAURI_REF__;
      if (tauri && tauri.core && typeof tauri.core.invoke === "function") {
        tauri.core
          .invoke("focus_main_shell", { x: Math.round(point.x), y: Math.round(point.y) })
          .catch(function (err) {
            console.warn("dsh-desktop: focus_main_shell failed:", err);
            onError();
          });
      } else {
        console.warn("dsh-desktop: captured __TAURI__ unavailable; focus_main_shell not sent");
        onError();
      }
    } catch (err) {
      console.warn("dsh-desktop: focus_main_shell failed:", err);
      onError();
    }
  }

  function inject() {
    if (document.getElementById(ROOT_ID)) {
      return;
    }

    var style = document.createElement("style");
    style.id = STYLE_ID;
    style.textContent = buildCss();
    document.head.appendChild(style);

    var root = document.createElement("div");
    root.id = ROOT_ID;

    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "dsh-launcher-btn";
    btn.title = "显示主工作台";
    btn.setAttribute("aria-label", "显示主工作台");
    btn.innerHTML = SVG;
    root.appendChild(btn);

    var lightTimer;
    btn.addEventListener("pointerdown", function () {
      btn.classList.add("dsh-launcher-pulled");
    });
    var release = function () {
      btn.classList.remove("dsh-launcher-pulled");
    };
    btn.addEventListener("pointerup", release);
    btn.addEventListener("pointerleave", release);
    btn.addEventListener("click", function (ev) {
      // Light optimistically; a failed invoke swaps warm for red.
      btn.classList.remove("dsh-launcher-err");
      btn.classList.add("dsh-launcher-on");
      clearTimeout(lightTimer);
      lightTimer = setTimeout(function () {
        btn.classList.remove("dsh-launcher-on");
        btn.classList.remove("dsh-launcher-err");
      }, LIGHT_MS);
      invokeFocusMainShell({ x: ev.screenX, y: ev.screenY }, function () {
        btn.classList.remove("dsh-launcher-on");
        btn.classList.add("dsh-launcher-err");
      });
    });

    document.body.appendChild(root);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", inject);
  } else {
    inject();
  }
})();