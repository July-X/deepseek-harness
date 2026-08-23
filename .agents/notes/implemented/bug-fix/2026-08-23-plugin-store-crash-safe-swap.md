# Agent Note: Crash-safe plugin store swap and stale-update-badge suppression

Status: implemented

English | [中文](2026-08-23-plugin-store-crash-safe-swap.zh.md)

## Problem

`plugins::fetch_into_store` ran the install/update swap as three fs renames in a row:

```text
rename(final_dir -> .old-*)
remove_dir_all(.old-*)
rename(tmp -> final_dir)
```

The middle `remove_dir_all` left a crash window where `final_dir` no longer existed, the old content was already gone, and the new content was still in `tmp`. A kernel panic, a `kill -9` on the Tauri shell, or a machine power-loss in that window produced a plugin store with no live plugin — `materialize_one` then could not link anything for any kernel, and the next `pnpm install` in the profile would error out because the link target was missing. The shell never noticed on its own; the user saw a "插件不存在" or a hard crash on the next kernel start.

Separately, `status()` filtered the top-level `updates` count through `is_newer_than`, but the per-row `latest_version` field was passed through verbatim to the UI. `desktop/ui/plugins.js` rendered the "有更新" badge from the field's truthiness, so a successful `update()` that synced `latest_version = installed_version` (the same string in both columns) still left the row carrying the badge and an "更新" button. The top-level "N 个更新" pill cleared correctly, which made the row-level ghost look like a renderer regression.

## Decision

**Three-stage staging names with a `.dsh-id` marker.** `fetch_into_store` now writes a `.dsh-id` marker inside every staging dir (whose name is `tmp-<pid>-<ts>`, `new-<pid>-<ts>`, `backup-<pid>-<ts>` — pid+ts prevent collisions across concurrent or crashed-then-resumed runs) and runs the swap as:

```text
fetch_git/fetch_npm   -> tmp-<pid>-<ts>
validate_plugin(tmp)  --+
                        |  ok
rename(tmp -> new)
rename(final -> backup)
rename(new -> final)
remove_dir_all(backup)
```

A crash at any step leaves the live plugin (`final_dir`) in one of two recoverable states: pointing at the previous version (untouched until the swap starts) or pointing at the new version (validation already passed before the publish rename). The transient gap when `final_dir` is briefly missing is reconciled on the next launch.

**`reconcile_store(data_dir)` runs unconditionally from `lib::setup`.** The recovery scan reads `~/.dsh/plugins/` once, groups staging dirs by `.dsh-id`, and applies the following table per plugin id:

| `final_dir` | `.new-*` | `.backup-*` | `.tmp-*` | action |
| --- | --- | --- | --- | --- |
| exists | any | any | any | remove all staging (post-publish cleanup or stale attempt) |
| missing | no | yes | no | revert: rename `.backup-*` → `final_dir` |
| missing | yes | no | no | publish: rename `.new-*` → `final_dir` |
| missing | yes | yes | any | revert (safer; user keeps the known-good previous version) |
| missing | no | no | yes | incomplete fetch; remove `.tmp-*` |

When multiple staging dirs share an id (recovery itself crashed), the freshest one wins by lexicographic suffix (`pid-ts` sorts newest-last); older peers are removed. The recovery leaves any staging without a `.dsh-id` marker untouched, so an older shell that wrote different staging names does not lose user data.

**`status()` filters the per-row `latest_version` field through `is_newer_than`.** The Rust side becomes the single source of truth for "what counts as an update": when `latest == installed`, the field becomes `None`, the top-level `updates` count stays at zero (its existing filter), and the UI's `if (row.latest_version)` check stops showing the ghost badge.

## Alternatives considered

**Use `renameat2(RENAME_EXCHANGE)` for an atomic swap on Linux.** Rejected for two reasons: macOS lacks it, and a platform-specific fallback would still need the same recovery scaffolding. The staging + reconcile design works uniformly across POSIX and the eventual Windows port.

**Recover by always promoting `.new-*` over `.backup-*`.** Rejected because the previous version is the one the user has actually exercised against their loaded profile; keeping it means a crashed-during-update user sees their plugin still work, with a one-click retry for the failed update. `.new-*` content is validated so promotion would also be safe, but revert is the strictly safer default.

**Detect staging leftovers via a marker file at the store root.** Rejected because `setup()` already owns store reconciliation as its first plugin-touching step, and the marker would still need the same scan to find which staging dirs exist. A single `read_dir` with prefix matching is cheaper than a write-and-check dance.

**Have the UI re-run `cmp_versions` for the per-row badge.** Rejected because the same comparison logic then lives in three places (Rust `is_newer_than`, Rust `cmp_versions`, JS port) and would drift. Filtering at the Rust boundary means the wire format (`latest_version: Option<String>`) already encodes "newer" and the UI cannot accidentally render stale data.

**Filter the badge with a CSS-only hide rule.** Rejected for the same reason — the JS check is `if (row.latest_version)`, and adding an inline `data-latest==installed` attribute and a CSS selector would still let future renderers render the wrong content. Fixing the data at the source fixes it everywhere.

## Consequences

- A plugin update that crashes mid-swap leaves the user with their previous working plugin (revert) or the just-validated new plugin (publish), never a missing plugin.
- `reconcile_store` is unconditional on every shell launch; the happy path is one `read_dir` with no work to do.
- `.tmp-*` is no longer reused as the post-validation staging name; the rename to `.new-*` is what signals "ready to publish" to the recovery scan.
- Plugin update failures (e.g. pnpm build still broken for other reasons) keep their original error path: `fetch_git` returns the error, `tmp` is removed, `final_dir` is untouched.
- The per-row `latest_version` field is now `None` whenever the recorded remote version is not newer than the installed one, so the UI cannot render a ghost badge even if its renderer is later rewritten to ignore the top-level count.
- The PATH-inheritance fix that landed in the immediately preceding commit (`fix(desktop): prepend pnpm's bin dir to child PATH for plugin builds`) is the prerequisite for `fetch_git`'s `pnpm install --enable-pre-post-scripts` step to actually reach `node` for `prepare`. Without it, `fetch_git` returns an error and `fetch_into_store` never gets past Phase 1, so the staging dir is removed before any swap happens. Together the two commits close both halves of the install-time failure modes users reported.
