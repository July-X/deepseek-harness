# Agent Note: Desktop shell performance fixes and code cleanup

Status: implemented

English | [中文](2026-08-23-desktop-shell-perf-cleanup.zh.md)

## Problem

The Windows build felt sluggish and had a residual console-window flash. An audit found the causes on both sides of the IPC boundary: sync Tauri commands doing heavy work on the main thread, one console-subsystem spawn missing `CREATE_NO_WINDOW`, and a UI poll loop that rewrote the DOM unconditionally.

## Decision

**Every blocking command runs on `spawn_blocking`.** Tauri executes non-async commands on the main thread, so `start_kernel` (worst case a full `pnpm install`), `stop_kernel`, `activate_version`, `remove_version`, `fetch_releases`, `get_status`, and `plugin_status` are now `async` commands delegating to blocking workers, following the existing `plugin_catalog` pattern. `State` cannot move into the closure, so the handlers carry `AppHandle` and re-acquire `app.state::<AppState>()` inside.

**`kernel::stop` polls `try_wait` with a one-second budget before SIGKILL.** The old code blocked in `wait()` after SIGTERM, making the SIGKILL line unreachable — an unresponsive kernel parked「关闭工作台」and app exit forever. The loop mirrors `kill_pid` in the same file.

**The `reg.exe` PATH probe goes through `quiet()`.** It was the one console-subsystem spawn left without `CREATE_NO_WINDOW` after `c22e3efa84`; the file's own comment already said it needed it.

**Polling is gated and write-minimal.** The 2.5s status interval skips while `document.hidden` and refreshes once on `visibilitychange`; a `setText` helper skips same-value `textContent` writes; `StatusView.kernel_log` was deleted because the UI never read it (logs use `get_kernel_log`). `promise_pnpm` takes the cached `NodeInfo` instead of re-spawning `node --version` per operation.

**Cleanup stayed adjacent to the touched code.** Shared pnpm args (`PNPM_REPORTER`, `PNPM_NO_STRICT_DEP_BUILDS`), `pnpm_spawn_err`, `http_get_string`/`http_get_bytes`, and one `run_plugin_command` runner replaced five near-identical plugin commands; the UI gained `el`/`mkBtn`/`armConfirm` helpers that absorbed ~13 repeated DOM-building blocks and two copies of the two-step delete confirm.

## Alternatives considered

**Event-push instead of polling for kernel status.** Rejected for now: the kernel is an external process the shell only observes, so liveness needs a probe regardless; gating the poll on visibility and keeping it off the main thread removes the measured cost without a protocol change.

**A deeper `plugins.rs` restructure.** Rejected: the remaining duplications there (install/update tails, semver parsing) are cheaper to lift during the next plugin-mechanism change than as a standalone refactor.

## Consequences

- `get_status` and `plugin_status` now return `Result<_, String>` at the command signature level; the success payload shape is unchanged, and a rejection means the blocking worker itself failed (a panic), which previously was impossible to observe.
- The `fail`/`done` labels in `withPluginProgress` now have fallbacks, so callers without labels (e.g. `updatePlugin`) no longer toast `undefined`.
