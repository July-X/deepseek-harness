# Agent Note: `fetch_npm` extracts into the staging dir, not a subdirectory it then deletes

Status: implemented

English | [中文](2026-09-01-desktop-npm-extract-target-dir.zh.md)

## Problem

Every plugin install from an npm tarball — npm origin plugins and git origin plugins whose prepare build calls `pnpm` against a published version — failed `validate_plugin` with `不符合 dsh 插件规范：缺少可解析的 package.json`. The flow was:

```rust
extract_tarball(&tgz, &dest.join("package"))   // -> dest/package/{package.json,lib/,...}
let _ = fs::remove_file(&tgz);
let _ = fs::remove_dir_all(dest.join("package"));   // <- deletes the extracted contents
Ok(version)
```

After `fetch_npm` returned, the staging dir `dest` held nothing but the deleted-tarball marker and any sibling files `git_latest_tag` / `build_git_plugin` had written for git-origin flows. `fetch_into_store` then called `validate_plugin(&tmp)` against an empty directory, hit the "no package.json" branch, and reported the misleading "missing manifest" error to the user. The bug has been latent since the initial community-plugin commit (7ebc6f5a352) on 2026-08-22 — it would have hit every platform equally; the reason it surfaced in this investigation is that the prior Windows PATH/tar fixes finally let the install get past the tar step and into `validate_plugin`, which is where the empty directory becomes visible.

A working `extract_tarball` is necessary but not sufficient: with `--strip-components=1` removing the npm tarball's leading `package/` segment, the plugin's `package.json`, `lib/`, `cordis.patch.yml`, … land inside whatever directory `tar -C` points at. Pointing that at `dest/package/` followed by `fs::remove_dir_all(dest.join("package"))` is a self-canceling move — the original code looked like it expected to clean up a temporary wrapper dir after moving the contents up one level, but no such move ever happened. The cleanup was always deleting the only copy of the extracted files.

## Decision

**Extract straight into the staging dir, drop the post-extract removal.** The fixed `fetch_npm` body is now:

```rust
let tgz = dest.join(".pkg.tgz");
fs::write(&tgz, bytes).map_err(|e| AppError::Io(e.to_string()))?;
extract_tarball(&tgz, dest).map_err(|e| AppError::Plugin(format!("解包失败：{e}（请确认系统存在 tar）")))?;
let _ = fs::remove_file(&tgz);
Ok(version)
```

`dest` is the `tmp-<pid>-<ts>` staging dir that `fetch_into_store` already created via `new_staging_dir`, so `extract_tarball`'s `fs::create_dir_all(dest)` is a no-op here and the subsequent `tar -C dest` lands the stripped contents at the root of the staging dir. `validate_plugin(&dest)` then reads `dest/package.json` directly, the `materialize_one` symlink/copy target picks up the real tree, and the `build_git_plugin` / `install_store_deps` flow that runs `pnpm install` against the same staging dir no longer operates on an empty workspace.

The tarball cleanup (`fs::remove_file(&tgz)`) stays — that one was always correct, it just got bundled with the broken package-dir removal in the diff.

## Consequences

- npm-origin plugin installs now succeed on every platform, not just Windows where the previous PATH/tar fix unblocked the upstream flow.
- `validate_plugin` no longer needs the "缺少可解析的 package.json" branch in its install-time surface; the branch stays in place for actual missing-manifest plugin authors, but the install path no longer trips it spuriously.
- The full install chain is reachable end-to-end: `fetch_npm` → `validate_plugin` → `materialize_one` → `install_store_deps` (link mode) → `sync_kernels` → `ensure_wiring`. None of those downstream steps needed changes because they always assumed the staging dir held the extracted tree — which is now finally true.
- `git` origin plugins were already working because `git clone` puts the tree at the clone target root, not under a subdir; the bug was strictly scoped to the npm path.

## Alternatives considered

- **Move the contents up one level instead of removing the subdir.** Rejected: that pattern requires a recursive copy or a directory rename across an existing target, and either way the staging dir ends up holding exactly what it would have held if we had just extracted there in the first place. The direct extraction is one fewer step with no rename-race window.
- **Extract into a different temp dir and rename it onto the staging dir.** Rejected: the staging dir already exists and is the canonical reference for every downstream step (and for the staging-dir reconcile tables). Renaming another dir onto it would need an empty source guarantee and would also fight with the `tmp-<pid>-<ts>` uniqueness check.
- **Make `validate_plugin` tolerant of a `package/` subdir.** Rejected: it would let the bug live, and any future tooling that expects `validate_plugin` to read `dir/package.json` (read by hand from the disk, scripted installs, the materialize target symlink) would silently pick the wrong path. The extraction target is the right place to be strict.