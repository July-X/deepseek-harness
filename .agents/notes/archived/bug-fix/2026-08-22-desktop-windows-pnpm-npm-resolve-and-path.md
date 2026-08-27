# Agent Note: dsh-desktop now finds Windows pnpm/npm shims and inherits the user PATH

Status: implemented
Archived: 2026-08-28

English | [中文](2026-08-22-desktop-windows-pnpm-npm-resolve-and-path.zh.md)

## Problem

Two interlocking Windows-only failures made the shell's auto-install path unusable on a default Chinese install. Both surface inside `node::ensure_pnpm` (kernel + plugin install bootstrap) as the user-visible "未检测到 pnpm，正在通过 npm 自动安装…安装失败：系统找不到指定的路径" sequence.

**Lookup wrong shape.** `resolve_pnpm` and `find_npm` walked the PATH looking for `<tool>.exe`. Node-adjacent tools installed by `npm install -g` on Windows are `.cmd` shims under the user's npm prefix (`%AppData%\npm\pnpm.cmd`, never `.pnpm.exe`), and the same is true for `npm` itself when a portable layout is used. The probe missed every user-prefix install and only happened to work when the tool sat next to `node.exe` (which is uncommon for `pnpm`).

**GUI process loses user PATH.** Tauri ships with `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`, which makes the runtime a GUI-subsystem binary. A GUI-subsystem process inherits the system path block only — `HKEY_CURRENT_USER\Environment\Path` (the value the user's npm/pnpm installers wrote to) is merged into the system path by Explorer and other interactive hosts, but never into what `CreateProcess` hands the GUI child at launch. `node.exe` (system-installed) is still findable, but anything the user installed afterwards is not. Release builds reproduce this 100%; debug builds inherit the parent's PATH and only sometimes hit the bug.

Together: `resolve_pnpm` returns `None` (no `.exe` on PATH), `ensure_pnpm` falls into the `npm install -g pnpm` branch, `process::spawn` cannot find `npm.cmd` either because the user PATH is missing, and the user sees an "os error 3" wrapped in a Chinese error message that does not name the real cause.

## Decision

**One Windows-aware helper, called from both `resolve_pnpm` and `find_npm`.** `node::which_in_dir(name, dir)` probes three candidate filenames per directory on Windows (`<name>.cmd`, `<name>.exe`, `<name>`) and just `<name>` on Unix. The `.cmd`-first order matches the layout every `npm install -g` produces; `.exe` keeps the standalone-install case working; the bare name survives the rare PATH entry that already includes an extension. `from_path` (which finds `node`) intentionally stays `.exe`-only — `node` itself is never a shim.

**Settings now carries an `npm_path` slot.** `Settings::npm_path: Option<String>` is checked first by `find_npm`, mirroring the existing `pnpm_path` slot. Defaults to `None`; existing `settings.json` files deserialize unchanged because the field is `#[serde(default)]`. The UI does not yet expose the slot, so the configuration is via direct edit of `~/.dsh/desktop/settings.json` for now — the manual-escape hatch matches the original `pnpm_path` design.

**Every spawned child now gets a merged PATH.** A new module `desktop/src-tauri/src/env.rs` reads `HKEY_CURRENT_USER\Environment\Path` once via `reg.exe query HKCU\Environment /v Path`, parses the `REG_SZ` / `REG_EXPAND_SZ` value, deduplicates against the inherited system PATH (case-insensitive, user entries first to match Explorer's concatenation order), and caches the result in a `OnceLock`. `process::spawn` stamps `cmd.env("PATH", env::merged_path())` on every `Command`, Windows and Unix alike; Unix returns the parent's PATH unchanged. `reg.exe` is invoked at a fixed `C:\Windows\System32\reg.exe` path with piped stdio and `quiet()` semantics so it never pops a console window, and `OnceLock` initialization keeps the read off the spawn hot path.

## Consequences

What shipped: `which_in_dir` helper +5 tests; `parse_reg_path` + `merge_paths` +5 tests covering the `reg query` output format and the dedup / user-wins ordering; `npm_path` settings slot with the existing serde-default round-trip; `env::merged_path()` applied to every `process::spawn` invocation (kernel installs, plugin store deps, profile wiring, git-origin plugin `prepare` builds, and the new fallback `npm install -g pnpm` itself). The shell now finds `pnpm.cmd` in the user prefix and inherits the user's PATH on every child process it spawns, so a Chinese-network install from a desktop-launched Tauri build works the same way it does from `cargo tauri dev`. The `find_npm` failure branch ("未检测到 pnpm，也未找到可用的 npm") still fires correctly when *neither* the configured path, the node sibling, nor any PATH entry holds an npm/pnpm binary — the upgrade is purely additive.

Known limitations. The registry read happens at process start; if the user installs a new tool mid-session the cached PATH misses it until restart (matches the existing `node_cache` pattern in `commands.rs`). `npm_path` is not yet surfaced in the UI; an editor of `settings.json` is the only way to set it today. `parse_reg_path` relies on `reg.exe`'s default text layout — a future Windows release that changes the column layout would need a follow-up. The PATH merge does not expand `REG_EXPAND_SZ` `%VAR%` references; child processes inherit them as-is and Windows expands them at lookup time, which matches what `cmd.exe` does.

## Alternatives considered

- **Keep `.exe`-only and tell users to set `pnpm_path`/`npm_path`** — fixes the symptom but the install path that needs them (`ensure_pnpm`'s fallback) is the one path that has not yet collected user config, so the user is stuck editing JSON before they can install anything.
- **Use the `which` crate** — pulls in a dependency for ~30 lines of file-probing and disagrees with `PATHEXT` ordering anyway (it walks extensions per directory, which is what we want, but its default `is_absolute()` filter rejects PATH-relative entries).
- **Switch the shell to console subsystem on Windows** — defeats `windows_subsystem = "windows"` and flashes a console window at every launch; no.
- **Read the user PATH via `winreg` crate at startup** — adds a dependency and complicates the read path; `reg.exe` ships at a fixed Windows path, does not need a crate, and is what every Windows shell-editor UI uses for the same lookup.
- **Spawn-time `expand_env` on `REG_EXPAND_SZ`** — would let `merged_path()` hold real paths instead of `%USERPROFILE%`-style references; deferred because the child shell already expands them at lookup, which is consistent with `cmd.exe`'s own behavior.
- **Expose `npm_path` in the UI alongside port / profile** — bigger change (HTML, JS, command handler, validation), not in scope for fixing the install failure; the JSON edit path is consistent with `pnpm_path`'s existing treatment.
