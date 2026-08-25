/**
 * Initialization script injected by `commands::open_harness` into the
 * dsh web workbench webview, alongside `titlebar-pulse.js`. It renders a
 * small pull-string lamp floating at the top-left of the workbench; pulling
 * (clicking) the string lights the bulb and brings the shell's main
 * management window to the foreground via the `focus_main_shell` command.
 *
 * Why inject from the shell rather than ship it in the kernel: the kernel
 * is a published npm artefact (`@deepseek-ai/dsh@<ver>`), and a shortcut
 * back to the shell's management panel is shell chrome, not workbench
 * content. Injecting from the shell keeps the widget present regardless of
 * which kernel version the workbench is running against.
 *
 * The widget talks back to the shell over `window.__TAURI__.core.invoke`
 * (tauri.conf.json sets `withGlobalTauri: true`), so it needs no kernel-side
 * endpoint and works on any kernel version. Because the workbench page is a
 * remote origin (`http://127.0.0.1:<port>`), Tauri's ACL only lets this
 * invoke through because `permissions/focus-main-shell.json` exposes the
 * command and `capabilities/harness-remote.json` grants it to the `harness`
 * window on the loopback origin.
 */
(function () {
  // Tauri runs initialization scripts in every frame; the widget belongs to
  // the top-level workbench document only.
  if (window.top !== window.self) {
    return;
  }

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
      /* Hang next to the sidebar header toggle (right of the brand logo).
         The offset is in CSS pixels, so display scaling and window zoom
         scale it together with the sidebar it aligns to; plain window
         resizes leave the fixed-width sidebar — and therefore the widget —
         unmoved. 212px puts the bulb's center at ~230px, right beside the
         sidebar-collapse button. */
      "  left: 212px;",
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
      /* Gitea green (#609926), matching the workbench chrome-row pulse in
         titlebar-pulse.js, so the string reads as workbench chrome. */
      "#" + ROOT_ID + " .dsh-launcher-cord {",
      "  stroke: #609926;",
      "  stroke-width: 2;",
      "  stroke-linecap: round;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-base {",
      "  fill: var(--dsh-launcher-base-fill);",
      "}",
      /* Palette variables, defaulting to the translucent whites that read
         against the dark workbench palette; the light-mode override near
         the end of the sheet swaps them. */
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
      /* Hover: the string brightens (lighter Gitea green) to read as
         grabbable. */
      "#" + ROOT_ID + " .dsh-launcher-btn:hover .dsh-launcher-cord {",
      "  stroke: #7dbd45;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn:hover .dsh-launcher-bulb {",
      "  stroke: var(--dsh-launcher-bulb-hover-stroke);",
      "}",
      /* Pulled: string + bulb travel down together, springing back on release. */
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
         ThemePresenter after): amber glass and darker linework keep the bulb
         legible on the white workbench. Only the palette variables change,
         so hover/lit precedence stays identical across themes. */
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
   * pixels); the shell moves the panel near it so the user does not have
   * to hunt for the window on another monitor. `onError` fires when the
   * Tauri IPC is unavailable or the command rejects, so the caller can
   * swap the warm glow for the red error flash.
   */
  function invokeFocusMainShell(point, onError) {
    try {
      var tauri = window.__TAURI__;
      if (tauri && tauri.core && typeof tauri.core.invoke === "function") {
        tauri.core
          .invoke("focus_main_shell", { x: Math.round(point.x), y: Math.round(point.y) })
          .catch(function (err) {
            console.warn("dsh-desktop: focus_main_shell failed:", err);
            onError();
          });
      } else {
        console.warn("dsh-desktop: window.__TAURI__ unavailable; focus_main_shell not sent");
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
