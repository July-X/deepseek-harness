# Agent Note: Desktop shell discovers nvm-managed Node installs

Status: implemented

English | [中文](2026-11-08-desktop-nvm-node-detection.zh.md)

## Problem

The desktop shell locates the Node runtime that runs `dsh web` through `desktop/src-tauri/src/node.rs::resolve`. The pre-fix search order — explicit `settings.node_path`, then a PATH walk, then a fixed set of well-known system locations (`/usr/local/bin/node`, `/opt/homebrew/bin/node`, `/usr/bin/node`, `C:\Program Files\nodejs\node.exe`, `%LOCALAPPDATA%\Programs\nodejs\node.exe`) — covered direct installs and Homebrew but missed every Node managed by [nvm](https://github.com/nvm-sh/nvm) (macOS/Linux: `$NVM_DIR/versions/node/vX.Y.Z/bin/node`) and [nvm-windows](https://github.com/coreybutler/nvm-windows) (`%NVM_HOME%/vX.Y.Z/node.exe`, exposed through `%NVM_SYMLINK%`).

A GUI shell inherits only the launchd PATH (`/usr/bin:/bin:/usr/sbin:/sbin`) on macOS and the Window-Station system PATH on Windows; user-installed locations added by `npm install -g`, Homebrew, and nvm live in `HKCU\Environment\Path` and the user's shell startup files, not in what a GUI subsystem process inherits at create-process time. The kernel install path therefore reported `未检测到 Node.js。请安装 Node.js 22.19+（或 >=24）后重试，或在设置中手动指定 node 路径。` and the only escape hatch was the manual path slot — viable, but most nvm users never reach it because the message does not name nvm.

A second, smaller class of failure followed once Node was found: `kernel::install_version` invoked pnpm with `extra_path_dirs = [pnpm_exe.parent()]`. When the user pinned pnpm via `settings.pnpm_path` and the resolved `node` lived elsewhere, pnpm's `#!/usr/bin/env node` shebang (and any lifecycle script that shells out to `node`) failed with `env: node: No such file or directory` even though the parent could spawn pnpm.

## Decision

Node auto-detection now scans the nvm-managed layouts directly. The PATH scan still runs first (a terminal-launched dev shell after `nvm use` lands the right version there), then nvm-managed installations, then the well-known system locations.

On macOS/Linux (`nvm-sh`), the shell resolves the nvm root from `$NVM_DIR` or `$HOME/.nvm`, enumerates `<root>/versions/node/<vX.Y.Z>/bin/node` directories, and resolves the `alias/default` file — following at most 5 hops so a hand-edited cycle cannot hang. The installation matching the resolved spec heads the probe list (exact match wins, then the newest installed version whose string extends the spec, mirroring how nvm interprets a bare-major alias like `22`). Remaining engine-compatible installations follow newest-first, with engine-incompatible ones filtered out before probing so each one costs at most a single child spawn.

On Windows (`nvm-windows`), the active junction (`%NVM_SYMLINK%`, what `nvm use` selected) heads the list, then every engine-compatible `<NVM_HOME>/vX.Y.Z/node.exe` under `%NVM_HOME%` and the default `%APPDATA%\nvm` installation newest-first. nvm-windows records its selection in the junction rather than an alias file, so there is no default spec to resolve there.

The empty-result message names both the engines range and the nvm-specific setup path (`nvm install 24 && nvm alias default 24`), and surfaces the closest failure it saw during the probe walk (a Node that runs but is too old) so the user can tell "no Node here" from "your Node is too old".

`kernel::install_version` takes a `node_dir: &Path` argument and prepends it to the child PATH before `pnpm_exe.parent()`. Any child shell spawned during the install (pnpm itself, lifecycle scripts that resolve `node`) sees the exact Node the parent used, regardless of where pnpm came from.

`detect_node` becomes `async` and runs `node::resolve` through `tauri::async_runtime::spawn_blocking`. Detection may spawn one child per environment candidate (PATH + nvm installs + system locations) until a usable Node is found; keeping the process spawns off the Tauri main thread follows the desktop conventions for commands that touch the filesystem and spawn children.

The detection result is split into two distinct failure messages. When no candidate exists anywhere on disk — fresh machine, no nvm, no system Node — `NO_NODE_FOUND_GUIDANCE` lists three independent install paths (a version manager — nvm, fnm, or volta — the platform package manager — Homebrew, NodeSource + apt, winget — and the official installer at [https://nodejs.org/](https://nodejs.org/)) plus the manual-path slot in「设置」. When a candidate runs but reports an engine-incompatible version, `NODE_TOO_OLD_GUIDANCE` lists only upgrade commands. The two messages do not overlap: fresh-install commands do not appear in the upgrade message, and upgrade commands do not appear in the no-Node message, because each user already knows whether they have a Node or not. `ensure_pnpm` shares the same no-Node guidance when neither `pnpm` nor `npm` is reachable, so the user gets a coherent set of installation options regardless of which command surfaced the failure.

## Alternatives considered

**Reading `HKCU\Environment` for NVM_HOME / NVM_SYMLINK on Windows.** Rejected for scope. The registry is the authoritative source when the GUI process loses the variables entirely, but adding a registry read here duplicates the merge logic already in `desktop/src-tauri/src/env.rs` and is rarely exercised because Explorer propagates user environment variables to processes it launches. The default `%APPDATA%\nvm` path covers the standard install; users with a non-default `NVM_HOME` typically see the env var propagated by the same Explorer that launched the shell, so the current scan already picks it up. Reopen only if a concrete failure shows the env var genuinely missing.

**Asking nvm directly via `bash -lc 'nvm which current'`.** Rejected. The shell has no expectation that the user has a POSIX shell available on Windows, the command requires `nvm.sh` to be sourced first, and the output is a script-path rather than a binary path. Direct disk scanning is platform-portable, does not require shell quoting, and works the same day a user installs nvm without re-launching a shell.

**Caching `node::resolve` by node_path only (current `cached_node` key).** Rejected for change. Detection results do depend on machine state (new nvm installs appear on disk immediately), but the status poll already runs every 2.5 s on a blocking worker, and a one-shot cache hit per `node_path` value is the documented trade-off. Re-keying the cache on a filesystem watcher is out of scope; the user can hit「检测 Node」 or change a setting to invalidate.

**Sourcing `~/.nvm/nvm.sh` from the launch agent so nvm is on PATH for every GUI child.** Rejected. macOS and Windows launchd/explorer already provide a stable environment at process creation; layering a shell-source step on top is fragile (file moves, login shell differences) and defeats the "no shell required" contract that direct disk scanning delivers.

**Treating any `node` on PATH as authoritative.** Rejected. A pre-fix PATH walk had exactly this behaviour and produced false-positive detection whenever the system PATH happened to contain an older Node (Homebrew default `node@18`, a leftover `/usr/bin/node`). Filtering incompatible ones out before the fallback and surfacing the closest failure in the message keeps the new path honest.

## Consequences

nvm users — historically a forced trip to the manual-path setting — install and run kernels from the management panel without extra steps on macOS, Linux, and Windows. A user who installs Node 24 via `nvm install 24` and then `nvm alias default 24` sees that version reflected in the next status refresh and in the install button's diagnostic. A user with multiple nvm versions and no `default` alias still gets the newest engine-compatible install.

The probe walks one child process per environment candidate until it finds a usable Node. In the common single-Node case the cost is a single `node --version` call, no change from the previous behaviour. With many installed nvm versions the walk is bounded by `compatible()` on the directory name (incompatible ones filtered before the probe), so the wall-clock cost stays small for typical 2–5 install setups.

The install path now reliably resolves `node` even when the user pinned a portable pnpm in `settings.pnpm_path` and runs nvm-managed Node, closing the `env: node: No such file or directory` class of failures inside `pnpm add`. `node::resolve` is the single source of truth; `commands::promise_pnpm` continues to feed it the resolved Node, and the `node_dir` parameter threads that into the kernel install without forcing `commands::install_kernel` to duplicate the resolution logic.

The empty-detection message names both the engines range and the nvm-specific setup path. Users on platforms with nvm but no installation see actionable guidance rather than a generic "install Node 22.19+" sentence that they cannot act on without leaving the application.

A fresh machine that has neither nvm nor Node installed now sees three independent install paths instead of one: a version manager (nvm / fnm / volta), a platform package manager (Homebrew on macOS, NodeSource + apt on Debian/Ubuntu, winget on Windows), and the official installer at [https://nodejs.org/](https://nodejs.org/), with the manual-path slot in「设置」 as the escape hatch. The detection result carries no near-miss in this case, so the message tells the user to install — not to upgrade — and skips upgrade commands that would not apply. When pnpm auto-install fails because `npm` itself is unreachable (which only happens when the user has neither pnpm nor npm on disk), the same no-Node guidance is appended, so the management panel and the install-progress panel both surface a consistent set of options.

## Testing

`cargo test --lib node::` covers the pure functions: alias-spec normalization, `format_version`, the partial-spec → highest-installed resolution (mirroring nvm), the incompatible-version drop, the unknown-spec fallback to desc-scan, and the alias-chain bounded follow (chain to a concrete version, partial spec when the alias target is not itself a file, cycle bounded by the hop cap, empty alias file). `resolve_alias` builds a fresh scratch directory under `std::env::temp_dir()` per test so parallel test runs do not collide.

`cargo test --lib` covers the broader surface (`process::merge_extra_path`, `settings`, `registry`, `version`, `env::parse_reg_path`, `env::merge_paths`). The two `plugins::tests` failures on Windows (`computes_relative_paths` path-separator mismatch, `materialize_link_then_copy` symlink fallback) are pre-existing on this branch and unrelated to this change.

`cargo clippy --all-targets` stays clean on the new code (one unrelated warning remains in `kernel::reap_orphans` and was already present before this change). `cargo fmt --check` is silent on the new code.