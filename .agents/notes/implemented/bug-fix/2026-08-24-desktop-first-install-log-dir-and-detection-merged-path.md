# Agent Note: dsh-desktop first-install log directory and merged-PATH tool detection

Status: implemented

English | [中文](2026-08-24-desktop-first-install-log-dir-and-detection-merged-path.zh.md)

## Problem

A first-run kernel install on a GUI-launched Windows shell died with "无法运行 npm 以自动安装 pnpm：系统找不到指定的路径 (os error 3)", reported from an nvm-windows machine where Node resolved fine but pnpm was genuinely absent. Two bugs stacked into that message, plus one latent bug on the recovery path.

**Log open failed before npm ever ran.** `ensure_pnpm` hands `process::run_with_progress` a log path under `<data_dir>/logs/`, but on a fresh data dir nothing has created `logs/` yet — `install_version` and `kernel::start` both create it later in the flow. `OpenOptions::open` failed with `NotFound` (Windows `ERROR_PATH_NOT_FOUND`, os error 3), and the wrap text blamed npm. Evidence on affected machines: zero `pnpm-install-*.log` files in the data dir, where any spawn failure would still have left an (empty) log behind.

**Detection still scanned the raw process PATH.** [The earlier Windows shim/PATH fix](2026-08-22-desktop-windows-pnpm-npm-resolve-and-path.md) merged the user PATH (`HKCU\Environment\Path`) into every spawned child, but `resolve_pnpm`, `find_npm`, and `from_path` kept scanning `std::env::var_os("PATH")`. A GUI-subsystem process inherits the system PATH only, so a `pnpm.cmd` the user already had under `%AppData%\npm` was invisible and the shell fell into the auto-install branch unnecessarily.

**Latent: the prefix fallback could not run on Windows.** `npm_prefix` spawned `npm` directly, but npm on Windows is a `.cmd` batch shim and CreateProcess cannot execute batch files — the post-install prefix probe was unreachable exactly for the users who needed auto-install.

## Decision

- `run_with_progress` creates the log file's parent directory before opening it; the contract is now that callers never pre-create the log directory.
- All PATH scans in `node.rs` go through one `path_dirs()` helper over `env::merged_path()`, so detection sees exactly the PATH the shell stamps onto spawned children.
- `npm_prefix` uses the new `process::script_output`, a one-shot sibling of `spawn` that routes `.cmd` shims through `%ComSpec% /C` and stamps the merged PATH with the tool's own directory prepended.
- The npm spawn-failure message now cites the full log path, per the desktop rule that user-facing errors name the log.

## Alternatives considered

- **Create `logs/` in `promise_pnpm` or at app setup** — fixes one caller while every other `run_with_progress` caller keeps the same latent failure; the helper owning its own log open is the single right place.
- **Expand `REG_EXPAND_SZ` `%VAR%` references before detection scans** — hand-rolled environment expansion for a rare storage form; unexpanded entries simply miss `is_file` and fall through, same as the child-PATH behavior recorded in the earlier note.
- **Leave detection on the process PATH and point users at the `pnpm_path` setting** — pushes a manual JSON edit onto every GUI user whose pnpm lives on the user PATH, when the shell already maintains the merged PATH for exactly this reason.

## Consequences

On the reported machine class (GUI-launched Windows shell, pnpm absent or on the user PATH), a first install now either finds the existing `pnpm.cmd` outright or actually runs `npm install -g pnpm`, with the transcript landing in `logs/pnpm-install-*.log`; under nvm-windows the global prefix is the user-writable version directory, so no elevation is needed and the freshly installed shim is visible through the `node_dir` probe immediately. The misleading "无法运行 npm" wrap can now only fire for genuine npm-launch failures, and it names the log to inspect. Verification: new `process::tests::run_with_progress_creates_missing_log_directory` regression test; `cargo check`, `cargo clippy --all-targets` (zero warnings), `cargo fmt`, and the lib test suite pass (two pre-existing `plugins::tests` Windows failures reproduce on the clean tree and are unrelated). The merged-PATH cache still initializes once per process, so a tool installed mid-session is detected only after restart — unchanged from the earlier note's recorded limitation.
