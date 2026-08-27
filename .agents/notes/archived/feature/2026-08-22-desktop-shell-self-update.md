# Agent Note: Desktop shell self-update via tauri-plugin-updater

Status: implemented
Archived: 2026-08-28

English | [中文](2026-08-22-desktop-shell-self-update.zh.md)

## Problem

The desktop shell could update the *kernel* (npm versions) but had no answer for updating *itself*: the overview page did not even show the shell's own version, and users had to watch the GitHub releases page manually. The update flow needed to cover auto-discovery, an on-demand check, and an in-app install that replaces the running app.

## Decision

**Self-update goes through `tauri-plugin-updater` against GitHub releases.** The release workflow signs updater artifacts (`latest.json` + `.sig` files) with the `TAURI_SIGNING_PRIVATE_KEY` repo secret; the pinned `pubkey` in `desktop/src-tauri/tauri.conf.json` makes the client reject anything not signed by that key. `bundle.createUpdaterArtifacts: true` produces the signed payloads on both targets.

**The endpoint is the latest published release.** `https://github.com/July-X/deepseek-harness/releases/latest/download/latest.json` serves only published releases — drafts are invisible, which matches the human-gated publish step. A prerelease (rc) can still be marked "latest" in the GitHub UI when rc builds should receive the next update.

**Discovery is push + pull.** `updater::spawn_background_check` runs once ~3s after launch and emits `shell-update-available`; the overview page also shows the running version (`StatusView.shell_version` from `app.package_info()`) with a「检查桌面端更新」button for manual checks. `install_shell_update` streams progress over a Channel, then `app.restart()`.

**NSIS stays `currentUser`.** The updater replaces the app in place without elevation only because the installer never required it.

## Alternatives considered

**Querying the releases API and launching the installer manually.** Rejected: no signature verification (any MITM or repo compromise ships arbitrary code to every desktop), two platform-specific replace paths to own, and no atomic swap. The updater plugin is the maintained path and its signature check is the security floor.

**A custom JSON endpoint outside GitHub releases.** Rejected: another artifact to host and keep in sync; the release's own `latest.json` cannot drift from the assets it describes.

## Consequences

- Losing `TAURI_SIGNING_PRIVATE_KEY` (kept locally at `~/.tauri/dsh-desktop.key` and in the fork's secrets) means updates stop verifying; rotate by generating a new keypair and updating the pinned pubkey plus the secret together.
- Version-to-tag consistency (`desktop-v<version>`) is what makes an update's version comparable; the workflow's verify step is the guard.
