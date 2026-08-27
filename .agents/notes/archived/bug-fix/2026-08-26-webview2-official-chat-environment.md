# Agent Note: WebView2 per-folder environment options and honest browser identity for official chat

Status: implemented
Archived: 2026-08-28

English | [中文](2026-08-26-webview2-official-chat-environment.zh.md)

## Problem

The `official-chat` window from [the Tauri desktop shell](../../archived/feature/2026-08-21-tauri-desktop-shell.md) failed against chat.deepseek.com in three escalating ways: the default configuration tripped the site's「使用环境异常」environment-check interstitial; the first hardening pass (custom browser arguments on the shared profile) stopped the window from being created at all; and the subsequent full Chrome-masquerade pass still left detectable inconsistencies between what the HTTP layer claimed and what the page could observe.

## Decision

**One user-data folder carries exactly one set of environment options.** WebView2 requires every environment created on a folder to agree on all options, additional browser arguments included. The panel and harness windows already run a default-options environment on the default folder, so any window needing custom args pins its own `.data_directory`; `open_official_chat` uses `<data_dir>/webview-official-chat`. Under mismatched options the second environment creation fails and the window never appears.

**Custom browser args replace wry's defaults.** Passing `additional_browser_args` drops the built-in `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`, so those entries are restated in `OFFICIAL_CHAT_BROWSER_ARGS` together with `AutomationControlled`, `TranslateUI`, `InterestFeedContentSuggestions`, and `--disable-blink-features=AutomationControlled`. Only the WebView2 backend consumes the args; macOS and Linux ignore them.

**The window presents its real Edge identity instead of faking Chrome.** A user-agent override cannot change `Sec-CH-UA` client hints or native `navigator.userAgentData`, so claiming Chrome in the header while hints report Edge creates the cross-layer mismatch environment checks flag; plain-JS shims also detectably differ from native platform objects such as `NavigatorUAData`. The builder leaves the UA untouched, disables the automation switches at the engine level, and reduces `chat-fingerprint.js` to two jobs: pin `navigator.webdriver` to `false` (the value a normal non-automated browser reports; `undefined` is itself a bot tell), and delete `__TAURI__` / `__TAURI_INTERNALS__` / `__TAURI_METADATA__` / `__TAURI_IPC__` outright.

## Alternatives considered

**Keep the Chrome masquerade and align the remaining signals.** Rejected: client hints cannot be rewritten from page script or through the user-agent setting, so the header-level contradiction is unavoidable without intercepting responses, and every shim added to hide one tell introduces another surface that differs from the native platform.

**Share the default folder but vary the browser args per window.** Rejected: environment creation fails under mismatched options on one folder — observed as the open button doing nothing once custom args were introduced.

**Keep exposing `window.__TAURI__` as a neutered Proxy so `typeof` checks pass.** Rejected: presence of any Tauri global is itself the embedded-webview signal; deletion reproduces exactly what a normal browser shows.

## Consequences

The three webviews coexist in one process with per-window option sets isolated by folder, and the dedicated profile adds a small directory under the shell data dir. The pull-string launcher keeps working because it captures `window.__TAURI__` before the deletion script runs. If chat.deepseek.com ever rejects honest Edge identities specifically, the remaining lever is response interception to align client hints with a claimed brand, which is deliberately not built.
