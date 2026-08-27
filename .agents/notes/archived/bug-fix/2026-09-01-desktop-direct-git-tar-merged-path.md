# Agent Note: dsh-desktop direct `git`/`tar` children inherit the merged PATH

Status: implemented
Archived: 2026-09-01

English | [中文](2026-09-01-desktop-direct-git-tar-merged-path.zh.md)

## Problem

The previous fix made the Windows GUI release build inherit the user PATH at every `process::spawn` site (kernel install, plugin store deps, profile wiring, npm/pnpm fallback, git-origin plugin `prepare` build). One branch was missed: the plugin store's git and tar invocations are short-lived helpers that bypass `process::spawn` and build their `Command` directly with `Command::new("git")` / `Command::new("tar")`. Those children inherit the GUI shell's launch-time PATH block — system only on Windows — and miss anything the user installed under `HKCU\Environment\Path`.

Concrete symptom on Windows: a user who installed Git for Windows via its standard installer (which writes `C:\Program Files\Git\cmd\` to the **user** PATH, not the system PATH) opens a desktop-launched `tauri build` artifact and tries to install a git-origin plugin. `fetch_git`'s `Command::new("git")` probe fails, the error surfaces wrapped as `未找到 git（git 来源的插件需要 git；请先安装 git）`, and the install aborts before reaching `git ls-remote` or `git clone`. The same shape would hit any other user-PATH-only tool we ever shell out to; `tar` happens to work today because Windows 10+ ships `bsdtar` at `C:\Windows\System32\tar.exe` (system PATH), but that luck is not something to depend on.

The pnpm-side note (`fix(desktop): prepend pnpm's bin dir to child PATH for plugin builds`, 75046c1a9d) and the Windows registry PATH merge (bfde8dd884) addressed the pnpm/npm and prepare-build flows; they did not cover the `Command::new` calls in `plugins.rs` because those do not go through `process::spawn` at all.

## Decision

**A single-shot helper `process::command_with_path(program)`.** It builds the `Command` and stamps `cmd.env("PATH", env::merged_path())` before returning, so every direct external tool invocation in the shell can route through one entry point and pick up the same merged PATH that `process::spawn` already provides to long-running helpers. On Unix the merge is a passthrough; on Windows it carries the user PATH the GUI subsystem would otherwise drop. The helper takes `S: AsRef<OsStr>` so callers can pass any program name (`"git"`, `"tar"`, or a future tool) without giving up the original `Command::new` ergonomics.

**Every direct external tool invocation in `plugins.rs` now goes through the helper.** Three sites moved:

- `run_capture` (used by `git_latest_tag` for `git ls-remote`).
- `fetch_git`'s `git --version` probe.
- `fetch_git`'s `git clone --depth 1` command.
- `extract_tarball`'s `tar -xzf … -C …` invocation.

`pnpm install`, `node` (kernel), and `npm install -g pnpm` already route through `process::spawn` and were not touched. The `Command` and `Stdio` imports in `plugins.rs` shrink to just `Stdio` (the `git clone` site still wants `Stdio::null()` for the silent `clone`); no other call site in that file constructs a `Command` directly.

**The new helper is exercised by a real spawn, not a Debug-format peek.** Rust's `Command` Debug output only reports the program and args (env entries live in an opaque internal table), so a unit test that only inspects `format!("{cmd:?}")` would not catch a missing `env` call. The added test spawns `cmd.exe /C "echo %PATH%"` (Windows) or `/bin/sh -c 'echo "$PATH"'` (Unix) through the helper, then asserts the child's echoed PATH is byte-identical to `env::merged_path()`. This proves the helper actually stamps PATH for the child rather than just looks like it might.

**`extract_tarball` pre-creates the destination directory and captures stderr into the error message.** The first user-PATH fix above is what got `tar.exe` found on Windows, but the same user hit a second failure right behind it: `tar -xzf … -C <dest>` exits 1 with `could not chdir to` when `<dest>` does not already exist. GNU tar creates the directory on demand; the Windows 10+ bsdtar shipped at `C:\Windows\System32\tar.exe` does not, and the prior error message (`退出码 Some(1)`) hid the actual cause. The fix is `fs::create_dir_all(dest)` before the spawn, plus piping stderr back so the next failure surfaces its real reason (corrupt archive, MAX_PATH overrun, permission denied, …) instead of an opaque exit code. stdout is discarded — bsdtar prints one extracted path per line and that noise does not belong in the install log.

**Pre-existing cross-platform test bugs in `process::tests` got fixed in the same change.** The `merge_extra_path_*` tests hard-coded Unix `:` as the separator; on Windows the helper produces `;`, so `cargo test --lib` failed there even before this change. Each test now derives its expected separator from `cfg!(windows)`. These failures did not block CI (the only test surface the Windows CI lane covers is `wine-windows-gates.sh` for the kernel packages), but a developer running the lib tests on Windows would see them, and the fix is two lines per assertion.

## Consequences

What shipped: `process::command_with_path` helper + 1 spawn-based test; `plugins.rs` rewires `run_capture`, `fetch_git`'s probe, `fetch_git`'s clone, and `extract_tarball` through the helper; `extract_tarball` now `mkdir -p`'s its destination and forwards tar's stderr into the error message; the helper is the documented one-stop for future direct tool invocations in the shell; pre-existing Windows test failures for `merge_extra_path_*` are corrected to track `cfg!(windows)`.

Together with the prior fixes this closes the Windows user-PATH surface for every tool the shell spawns:

- pnpm/npm shims under the user npm prefix → `process::spawn` (already correct via `env::merged_path`).
- `git` / `tar` / future direct tools → `process::command_with_path` (new).
- The Windows bsdtar `tar -C` quirk → `fs::create_dir_all` before spawn (new).
- `node` (kernel entry) and the npm install-g-pnpm fallback already inherit the merge via the same `process::spawn` and `node::ensure_pnpm` paths.

A Windows user with the standard Git for Windows install (user PATH only) can now install both npm and git-origin plugins from a `tauri build` release artifact. The pre-existing "未找到 git" error still fires correctly when git genuinely is missing — the probe still calls `git --version`, the helper only changes which PATH it looks under. NPM tarball extraction no longer fails with the opaque `退出码 Some(1)` when the staging directory is fresh — and when something else goes wrong (corrupt archive, long paths, ACL denial), the user sees the actual bsdtar diagnostic instead of an exit code.

Two pre-existing Windows-only test failures in `plugins.rs` (`computes_relative_paths` and `materialize_link_then_copy`) remain untouched: they are test-harness bugs unrelated to this fix and are best addressed in their own change so this commit's diff stays focused on the user-PATH surface.

Known limitations. The PATH merge is cached at process start (`OnceLock`); a user who installs git during the shell's lifetime still has to restart the shell to pick up the new entry (matches the existing `node_cache` pattern). `command_with_path` only sets `PATH`; if a future tool needs additional env entries (`GIT_TERMINAL_PROMPT`, `CARGO_TERM_COLOR`, …) the call site still has to set them explicitly. The shell does not add `extra_path_dirs` here — that path is reserved for `process::spawn` because the only known consumer is pnpm's bin dir for lifecycle-script `node` resolution.

## Alternatives considered

- **Route `git` and `tar` through `process::spawn` instead.** Rejected: those are quick, fire-and-forget commands that do not need the dual-stream drain or heartbeat, and adapting `spawn` to take a `fire-and-forget: true` knob would expand its contract for a single benefit. The single-shot helper is a few lines and reuses the same `env::merged_path` source.
- **Bake the PATH stamp into `Command::new` via a wrapper type.** Rejected: every Rust call site that already says `Command::new(...)` would need to learn the new type, and the `Command` API is wide enough that a wrapper would leak in the first non-trivial composition. A free function is enough.
- **Document "use `process::spawn` instead of `Command::new`" and call it done.** Rejected because `process::spawn` requires a log path and progress callback; `tar -xzf` and `git ls-remote` neither want a log file nor a heartbeat. Forcing them through `process::spawn` would be ceremony for ceremony's sake.
- **Set `PATH` in `setup()` so every Command inherits it for free.** Rejected: `process::Command` reads the parent process's PATH at `spawn` time, not at `Command::new` time, and the parent process PATH on Windows is still system-only. Mutating `std::env` directly is also `unsafe` in a multi-threaded Tauri runtime (the `OnceLock` pattern in `env.rs` exists precisely to avoid that).
- **Ship a registry-less fallback that lets the user point at `git.exe` manually via settings.** Deferred: would mirror the existing `pnpm_path` / `npm_path` slots, but every additional manual-config slot is a worse user experience than "use the user's PATH like every other GUI shell does". The first failure path the user hits should be a fixable PATH, not a settings.json edit.