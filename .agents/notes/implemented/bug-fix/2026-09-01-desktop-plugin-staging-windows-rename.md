# Agent Note: plugin staging-dir swap survives Windows non-empty rename targets and cleans up synchronously

Status: implemented

English | [中文](2026-09-01-desktop-plugin-staging-windows-rename.zh.md)

## Problem

After the prior fixes (PATH inheritance, tar `-C` directory, fetch_npm target), the install flow reached `fetch_into_store` and then hit `I/O 错误：目录不是空的。 (os error 145)` from `fs::rename`. Two interacting bugs:

**`new_staging_dir` was both redundant and unsafe on Windows.** The function wrote `.dsh-id` into the staging dir immediately after creating it, then the swap renamed a content-bearing `tmp` onto a `new` whose only resident was a single file. On Unix, `rename(2)` replaces directories atomically, so this works. On Windows, `MoveFileEx` rejects a non-empty target directory with `ERROR_DIR_NOT_EMPTY` (os error 145); the rename fails and the user sees an opaque `I/O 错误`. The same shape hit any second attempt at the swap when a previous failure had left a `.new-*` dir with the marker still inside, because the new helper's `let _ = fs::remove_dir_all(&dir);` swallowed the cleanup error and then `fs::create_dir_all(&dir)?` returned `Ok(())` on a non-empty existing dir, handing the caller a "fresh" dir full of stale content.

**The post-publish backup cleanup was best-effort and silent.** `let _ = fs::remove_dir_all(&backup);` after the swap meant a failed cleanup left a `.backup-*` dir behind with no surfaced error. Combined with the rename failures above, the user's store could accumulate dirty staging dirs across multiple install attempts, each one a future `ERROR_DIR_NOT_EMPTY` trigger.

The bug has been latent since the initial community-plugin commit (7ebc6f5a352) on 2026-08-22 — but, as with the prior fix, Windows was the only platform the user could actually see it on. macOS and Linux's `rename(2)` happily replaces non-empty directories, so the same code path "worked" there by accident. The reason the user hit it now is that the previous fixes finally let the install reach the swap, exposing what was always going to be a Windows-only failure mode.

## Decision

**`new_staging_dir` no longer writes the marker; it returns an empty dir.** The marker is now a separate `stamp_id_marker(dir, id)` call. The caller is responsible for stamping the rename *source* before the swap (`tmp` in `fetch_into_store`); the rename target (`new`, and the implicit `backup` target) stays empty until the rename succeeds, after which the marker either rides along from `tmp` (the `new` case) or is stamped explicitly (the `backup` case, since `final_dir` does not carry a marker). On Windows the rename always lands on an empty directory and `MoveFileEx` accepts it.

**Timestamps step up from seconds to nanoseconds** so two `new_staging_dir` calls in the same second (the previous failure case for a quick retry after a crash) no longer collide on `tmp-<pid>-<ts>` vs `tmp-<pid>-<ts>`. The cleanup is now a safety net rather than a load-bearing step: collisions in normal operation are statistically impossible, and a stuck cleanup still surfaces through the helper's return value.

**`new_staging_dir` propagates `remove_dir_all` errors** instead of swallowing them. Anything other than `NotFound` is returned to the caller as `AppError::Io`. The user's failure no longer gets hidden behind an `Ok(())` "yes I cleaned the dir" that actually left the dir dirty.

**The swap in `fetch_into_store` now rolls forward or back on rename failure** and surfaces the situation to the user:

- `tmp → new` fails: the validated content is left on disk in `tmp`; the user sees `将暂存目录提升到 .new-* 失败：<detail>`. A retry / `reconcile_store` on next launch can promote it.
- `final_dir → backup` fails: the new content is forwarded by promoting `new` to `final_dir` (so the install actually takes effect), and the user sees `插件已发布，但备份旧版本失败（<detail>）；下次更新若失败将无法回滚`. The partial-success warning is loud.
- `new → final_dir` fails after backup succeeded: the previous live plugin is restored from `backup` back to `final_dir`, and the user sees `发布新版本失败，已回滚到旧版本：<detail>`. `new` becomes a stranded `.new-*` for `reconcile_store` to pick up next launch.
- `new → final_dir` fails and rollback also fails: the user sees `发布新版本失败且回滚旧版本失败：<detail>` and `reconcile_store` is the recovery path on next launch.

**The post-publish backup cleanup is synchronous and surfaces failures.** A user-facing error is returned if `fs::remove_dir_all(&backup)` fails, naming `reconcile_store` as the next-launch repair path. Stale `.backup-*` dirs can no longer accumulate silently.

**`write_source_marker` is called after the swap and after the cleanup.** The marker file (`.dsh-source.json`) is written to `final_dir` only once the rename has succeeded and the temp dirs are gone. This way, on any error path, `final_dir` either does not exist yet (no plugin) or still has the previous version with its own marker (recovery via `reconcile_store`).

## Consequences

- npm-origin and git-origin plugin installs no longer hit `ERROR_DIR_NOT_EMPTY` on Windows during the staging swap.
- Each rename in `fetch_into_store` has a defined recovery path; the previous "rename fails → silent" surface is gone.
- The post-publish backup is gone before the function returns success, so the user does not need to wait for `reconcile_store` to clean up after a happy install.
- A failed `tmp → new` rename leaves the validated content in `tmp` rather than in `new`. The `reconcile_store` rules on next launch already cover "live plugin present + tmp orphan" (just discard tmp) and "live plugin missing + tmp + …" (depending on which siblings survive), so this state is recoverable.
- macOS/Linux behaviour is unchanged in the happy path. The new error reporting is platform-agnostic and just makes the failure mode visible everywhere instead of silently recovering via `rename(2)` magic.

The two pre-existing Windows-only test failures (`computes_relative_paths` and `materialize_link_then_copy`) remain untouched and are still slated for their own commit.

## Alternatives considered

- **Revert the swap to a simpler "extract to `tmp`, then rename `tmp → final_dir` directly".** Rejected: the three-stage swap is load-bearing for crash safety — a crash mid-rename without `backup` would leave the user with no plugin and no recovery point, and `reconcile_store` is built around the three names. The fix is in the implementation of the three-stage swap, not its shape.
- **Use `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING` to replace non-empty directories on Windows.** Not exposed by `std::fs::rename` and would require pulling in `windows-sys` or a thin FFI wrapper. The empty-target design avoids the whole compatibility question and works on every platform.
- **Switch from `fs::rename` to a copy-then-delete dance.** Rejected: copy is slow on the plugin size, and a crash mid-copy leaves the user with two half-copies that `reconcile_store` cannot tell apart. The empty-target design is one extra helper plus a post-rename stamp, no copy at all.
- **Use a single shared staging dir for every plugin.** Rejected: plugin id is the identity, and a shared dir would have to be re-validated per install. The per-id naming is what makes `reconcile_store` able to group orphan dirs back to their plugin.
- **Make `reconcile_store` also run on every install start, not just shell startup.** Rejected as the primary fix because the user's install is already failing and we want the install path itself to be self-healing. `reconcile_store` is the safety net for crashes, not for live failures. The new error reporting makes a future pre-install cleanup a small additive change if we ever want it.