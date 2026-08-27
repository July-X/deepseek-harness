/**
 * Initialization script injected into both local webviews the shell owns:
 *
 * - `open_harness` (the dsh web workbench webview, served from the kernel
 *   over `http://127.0.0.1:<port>`).
 * - `open_official_chat`'s strip webview (label `official-chat-strip`, the
 *   local SPA route `index.html?chatstrip=1` that renders the tab bar).
 *   The lamp is hosted by the SAME strip HWND; the SVG is now a compact
 *   24x38 desk-lamp that fits inside the natural 38px tab-bar height, so
 *   the strip does not need to be inflated (an earlier 66px SVG forced
 *   the strip up to 66px and left a dark-blue empty band under the tabs).
 *
 * Both webviews keep `window.__TAURI__` intact — neither has its IPC
 * surface neutered — so the lamp talks straight to the global and the
 * injection only needs to handle chrome geometry, not fingerprint
 * defences.
 *
 * Pulling (clicking) the string lights the bulb and brings the shell's
 * main management window to the foreground via the `focus_main_shell`
 * command. The widget talks back to the shell over
 * `window.__TAURI__.core.invoke` (tauri.conf.json sets
 * `withGlobalTauri: true`), so it needs no page-side endpoint.
 *
 * Why inject from the shell rather than ship it in the kernel: the kernel
 * is a published npm artefact (`@deepseek-ai/dsh@<ver>`), and a shortcut
 * back to the shell's management panel is shell chrome, not page
 * content. The official-chat content webviews (DeepSeek / 千问 / MiniMax)
 * are third-party origins we cannot ship scripts into; the lamp itself
 * is window-level chrome and lives on the strip webview, which the shell
 * owns.
 *
 * Two surfaces, two anchors and palettes:
 *
 * - dsh web workbench: Gitea green cord (#609926) at left:212px, sized
 *   to land beside the sidebar-collapse button (the comment near the
 *   offset documents the geometry in detail).
 *
 * - official-chat strip: DeepSeek blue cord (#4D6BFE) at right:12px,
 *   mirrored to the right edge — the strip has no right-side content to
 *   collide with, and mirroring keeps the lamp visible regardless of how
 *   many tab buttons the strip grows.
 */
(function () {
  // Tauri runs initialization scripts in every frame; the widget belongs to
  // the top-level page only — without this guard an iframe inside the
  // workbench would mount its own lamp and double-paint the icon on every
  // nested document. The strip page is a single document so the guard is a
  // harmless no-op there.
  if (window.top !== window.self) {
    return;
  }

  // Surface selection: the official-chat strip webview (loaded at
  // `?chatstrip=1`) uses the DeepSeek-blue right-edge chrome; the
  // dsh web workbench has no such param and uses the Gitea-green left
  // anchor beside the sidebar-collapse button. Detecting by query keeps
  // the lamp purely local and means we never have to special-case
  // third-party origins here. (The lamp was briefly hosted by a separate
  // `?chatlauncher=1` webview, but WebView2 child HWND transparency
  // isn't reliable on this Tauri 2.11 stack so the lamp moved back into
  // the strip — `chatlauncher` is no longer a route, but the substring
  // check is kept as a no-cost safety net in case any old URL ever
  // leaks into the launcher route.)
  var search = window.location.search;
  var isOfficial =
    search.indexOf("chatstrip=1") !== -1 ||
    search.indexOf("chatlauncher=1") !== -1;
  var PALETTE = isOfficial
    ? {
        cord: "#4D6BFE",
      }
    : {
        cord: "#609926",
      };
  // 官方对话窗口的拉绳灯挂在右侧边缘（right:12px，与原先的左侧 left:12px 镜像
  // 对称）；dsh web 工作台仍贴左侧 left:212px（在品牌 logo 右侧、侧栏折叠键旁）。
  // SVG（绳/底座/灯泡/灯丝）全部以 x=12 为中心左右对称，故仅切换锚定边即可，
  // 无需水平翻转图形。
  var SIDE = isOfficial ? "right" : "left";
  var EDGE_PX = isOfficial ? "12px" : "212px";
  // Both surfaces the lamp lives on have enough vertical room for the
  // 38px compact SVG: the dsh web workbench is a full-window webview
  // (no top clip), and the official-chat strip is its natural 38px
  // tab-bar height (the SVG is sized to fit it exactly). `top: 0`
  // therefore anchors the lamp at the top edge of the viewport in both
  // surfaces, reading as a small desk lamp sitting on the chrome.
  var TOP_PX = "0px";

  var ROOT_ID = "dsh-shell-launcher";
  var STYLE_ID = "dsh-shell-launcher-style";
  // Small desk-lamp SVG that fits inside the 38px-tall official-chat
  // strip. The original pull-string lamp (66px tall) had to live in a
  // 66px-tall HWND; this compact rewrite (24x38) reads as a small
  // desk lamp — short pull chain hanging from the top, conical shade,
  // tiny bulb peeking out under the shade, thin stem, rounded base.
  var SVG = [
    '<svg viewBox="0 0 24 38" width="24" height="38" aria-hidden="true">',
    /* Pull chain (short, from the top of the strip down to the shade). */
    '<line class="dsh-launcher-cord" x1="12" y1="2" x2="12" y2="5"/>',
    /* Lamp shade (trapezoid: narrower at top, wider at bottom). */
    '<polygon class="dsh-launcher-shade" points="9,5 15,5 17,13 7,13"/>',
    /* Bulb peeking out under the shade. */
    '<circle class="dsh-launcher-bulb" cx="12" cy="14" r="2"/>',
    /* Stem. */
    '<line class="dsh-launcher-stem" x1="12" y1="16" x2="12" y2="30"/>',
    /* Rounded base. */
    '<rect class="dsh-launcher-base" x="3" y="30" width="18" height="6" rx="1.5"/>',
    "</svg>",
  ].join("");

  function buildCss() {
    return [
      "#" + ROOT_ID + " {",
      "  position: fixed;",
      "  top: " + TOP_PX + ";",
      /* Hang at the chrome corner of the page: left:212px on the dsh web
         workbench (right of the brand logo, beside the sidebar-collapse
         button), right:12px on the official-chat strip (the strip has
         nothing on its right side, so 12px from the edge reads as
         "flush right"). Plain window resizes leave both anchors alone —
         the workbench's sidebar is fixed-width and the strip is its
         natural 38px tab-bar height, both wide enough that the lamp
         has clearance. `top` is `0` on both surfaces because the
         compact 38px desk-lamp fits in the strip HWND without
         clipping. */
      "  " + SIDE + ": " + EDGE_PX + ";",
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
      "  transition: fill 0.18s ease;",
      "}",
      /* Conical shade of the desk lamp: same translucent glass as the
         bulb, with a thin lighter linework to read as a defined shape
         against the dark strip. */
      "#" + ROOT_ID + " .dsh-launcher-shade {",
      "  fill: rgba(255, 255, 255, 0.18);",
      "  stroke: rgba(255, 255, 255, 0.55);",
      "  stroke-width: 0.8;",
      "  stroke-linejoin: round;",
      "  transition: fill 0.18s ease, stroke 0.18s ease, stroke-width 0.18s ease;",
      "}",
      /* Stem is a single 1px line connecting the shade to the base. */
      "#" + ROOT_ID + " .dsh-launcher-stem {",
      "  stroke: rgba(255, 255, 255, 0.55);",
      "  stroke-width: 1;",
      "  stroke-linecap: round;",
      "  transition: stroke 0.18s ease, stroke-width 0.18s ease;",
      "}",
      /* Palette variables, defaulting to the translucent whites that read
         against the dark page palette; the light-mode override near the
         end of the sheet swaps them. */
      "#" + ROOT_ID + " {",
      "  --dsh-launcher-base-fill: rgba(255, 255, 255, 0.42);",
      "  --dsh-launcher-bulb-fill: rgba(255, 255, 255, 0.14);",
      "  --dsh-launcher-bulb-stroke: rgba(255, 255, 255, 0.55);",
      "  --dsh-launcher-lit-fill: #ffd45e;",
      "  --dsh-launcher-lit-stroke: #ffdf8a;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-bulb {",
      "  fill: var(--dsh-launcher-bulb-fill);",
      "  stroke: var(--dsh-launcher-bulb-stroke);",
      "  stroke-width: 0.8;",
      "  transition: fill 0.18s ease-out, stroke 0.18s ease-out, stroke-width 0.18s ease-out;",
      "}",


      /* Pulled: cord + shade travel down together, springing back on
         release. 3px is proportional to the 38px compact lamp. */
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-pulled svg {",
      "  transform: translateY(3px);",
      "}",
      /* On (click-toggled): the lamp glows. The shade picks up a warm
         fill + bright warm stroke (lit from inside the bulb), the bulb
         goes solid warm yellow, the stem + base warm up slightly, and
         two stacked drop-shadows give the icon a clearly bigger halo
         than its off-state depth shadow. No blink, no stroke-width
         jump — just a steady glow that persists until the next click. */
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-on .dsh-launcher-shade {",
      "  fill: rgba(255, 212, 94, 0.32);",
      "  stroke: rgba(255, 212, 94, 0.85);",
      "  stroke-width: 1.2;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-on .dsh-launcher-bulb {",
      "  fill: #ffd45e;",
      "  stroke: #ffdf8a;",
      "  stroke-width: 1.2;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-on .dsh-launcher-stem {",
      "  stroke: rgba(255, 212, 94, 0.7);",
      "  stroke-width: 1.2;",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-on .dsh-launcher-base {",
      "  fill: rgba(255, 255, 255, 0.55);",
      "}",
      "#" + ROOT_ID + " .dsh-launcher-btn.dsh-launcher-on svg {",
      /* Stack: small depth shadow + tight 12px warm glow + wide 24px
         softer glow. The two warm layers together make the icon's upper
         half (shade + bulb) read as the light source. */
      "  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.45))",
      "          drop-shadow(0 0 12px rgba(255, 212, 94, 0.95))",
      "          drop-shadow(0 0 24px rgba(255, 180, 50, 0.55));",
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
   * Both surfaces the lamp lives on (the workbench webview and the
   * official-chat strip webview) keep `window.__TAURI__` intact, so a
   * direct read is correct: there is no fingerprint script to neuter
   * the bridge between injection and click time.
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
        console.warn("dsh-desktop: __TAURI__ unavailable; focus_main_shell not sent");
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

    // Click toggles the lamp on/off (no auto-fade). focus_main_shell is
    // still invoked on every click to raise the management panel to the
    // front; on IPC failure the lamp drops back to off and shows a red
    // error indicator until the next click.
    var isOn = false;
    btn.addEventListener("pointerdown", function () {
      btn.classList.add("dsh-launcher-pulled");
    });
    var release = function () {
      btn.classList.remove("dsh-launcher-pulled");
    };
    btn.addEventListener("pointerup", release);
    btn.addEventListener("pointerleave", release);
    btn.addEventListener("click", function (ev) {
      isOn = !isOn;
      btn.classList.remove("dsh-launcher-err");
      btn.classList.toggle("dsh-launcher-on", isOn);
      invokeFocusMainShell({ x: ev.screenX, y: ev.screenY }, function () {
        isOn = false;
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