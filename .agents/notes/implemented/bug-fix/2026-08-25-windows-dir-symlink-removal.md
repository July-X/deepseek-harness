# Agent Note: Remove Windows directory symlinks with RemoveDirectory

Status: implemented

English | [中文](2026-08-25-windows-dir-symlink-removal.zh.md)

## Problem

The desktop shell materializes store plugins and skills into kernels and the skills root as directory symlinks. Re-materializing one on Windows (update, sync, mode switch) failed with `复制插件到内核失败：系统找不到指定的文件 (os error 2)` and left dangling links, orphaned store dirs, and unwired profile manifests behind. The chain: `DeleteFile` rejects directory symlinks with `ERROR_ACCESS_DENIED` — only `RemoveDirectory` removes them — so the swallowed `let _ = fs::remove_file(..)` left the link in place; link creation then failed with `ERROR_ALREADY_EXISTS` and fell back to copy; the copy followed the surviving link and copied the store directory onto itself. One plugin's failure also aborted the whole wiring pass, so uninstall residue was never pruned and the store warning stuck forever. The same `remove_file`-on-dir-symlink pattern existed in the skills module.

## Decision

`desktop/src-tauri/src/plugins.rs` and `skills.rs` each own a `remove_link` helper that tries `fs::remove_file` and falls back to `fs::remove_dir`, covering file and directory links on every platform (unix `unlink` handles both, so the fallback is Windows-only in practice). Around that root fix, the plugin materialization path is hardened as one change: `copy_tree` reports the exact failing path in every IO error, skips dangling links instead of aborting, and detects link cycles (circular pnpm dependencies on macOS/Linux, where `node_modules` is all symlinks) by comparing each directory's canonical path against the recursion stack — diamonds are not cycles and pass. Wiring no longer aborts on one plugin's failure: failed plugins are aggregated into one error while healthy plugins wire and uninstall residue prunes normally. A sweep removes kernel plugin entries the store no longer holds when they are shell-owned (meta record) or already broken (dangling link). `uninstall` deletes the store directory first and reports lock failures with a close-the-workbench hint instead of leaving an orphan. `reconcile_store` reaps staging dirs without an id marker (including the legacy `.tmp-` naming) while sparing live plugins whose names carry a staging prefix. `skills.rs` additionally recognizes `\\?\` verbatim paths so updating a local skill package works on Windows.

## Alternatives considered

**Platform-cfg'd removal (`remove_dir` on Windows, `remove_file` elsewhere).** Rejected: the link kind (file vs directory) is not known at the call sites, and try-both is one portable helper with no cfg surface.

**A recursion depth cap instead of canonical cycle detection.** Rejected: on Windows, paths past `MAX_PATH` fail `metadata` with the same `NotFound` as a genuinely dangling link, so the cycle was silently skipped as "dangling" before any cap fired; canonical ancestor comparison catches the cycle at the second recursion level on every platform.

**Abort wiring on the first failing plugin (status quo).** Rejected: the failure already destroyed that plugin's materialization, and aborting blocked the manifest pruning that removes uninstalled residue — the exact compounding the incident showed.

## Consequences

Update/sync/mode-switch on Windows no longer corrupts plugin or skill materializations, and a single broken plugin degrades to a named warning instead of a wedged wiring pass. `copy_tree` is slower by one `canonicalize` per directory. The link-removal rule complements the junction-fixture rule in [Unlink fixture junctions before recursive deletion](2026-08-12-unlink-fixture-junctions-before-delete.md): that note owns recursive deletion following links into targets; this one owns removing the link itself. Unit tests pin link-then-copy rematerialization, dangling-link skip, cycle abort, orphan sweep, unmarked-staging reaping, prefix-named live plugins, and wiring survival over a single plugin failure on both unix and Windows.
