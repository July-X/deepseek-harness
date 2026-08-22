//! Shell self-updates via tauri-plugin-updater, against the latest published
//! GitHub release's `latest.json` (see tauri.conf.json `plugins.updater`).
//!
//! The release workflow signs the updater artifacts with the
//! `TAURI_SIGNING_PRIVATE_KEY` repo secret; the public key pinned in the
//! config rejects any payload not signed by it. The endpoint serves only
//! published releases (a draft is invisible), so an update appears here once
//! a human publishes the draft — and only when that release is marked
//! "latest", which GitHub allows for prereleases.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

use crate::error::AppError;

/// What the overview page shows about the shell's own version state.
#[derive(Debug, Clone, Serialize)]
pub struct ShellUpdateInfo {
    pub current: String,
    /// Version of the published update, when one exists.
    pub available: Option<String>,
}

/// Event emitted when a background check finds a newer shell release.
pub const UPDATE_AVAILABLE_EVENT: &str = "shell-update-available";

/// Compare the running version against the latest published release.
pub async fn check(app: &AppHandle) -> Result<ShellUpdateInfo, AppError> {
    let current = app.package_info().version.to_string();
    let update = app
        .updater()
        .map_err(|e| AppError::Update(format!("初始化失败：{e}")))?
        .check()
        .await
        .map_err(|e| AppError::Update(format!("检查失败：{e}（需要网络与已发布的 release）")))?;
    Ok(ShellUpdateInfo {
        current,
        available: update.map(|u| u.version),
    })
}

/// One startup check shortly after launch; findings reach the UI as an event
/// so the overview page can raise the update banner without user action.
pub fn spawn_background_check(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Give the window a moment to mount its listener before emitting.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if let Ok(info) = check(&app).await {
            if let Some(version) = info.available {
                let _ = app.emit(UPDATE_AVAILABLE_EVENT, version);
            }
        }
    });
}

/// Download the pending update, install it, and restart into the new
/// version. The updater verifies the minisign signature against the pinned
/// pubkey before anything is replaced.
pub async fn install(
    app: &AppHandle,
    on_progress: impl FnMut(&str) + Send,
) -> Result<(), AppError> {
    let update = app
        .updater()
        .map_err(|e| AppError::Update(format!("初始化失败：{e}")))?
        .check()
        .await
        .map_err(|e| AppError::Update(format!("检查失败：{e}")))?
        .ok_or_else(|| AppError::Update("当前已是最新版本".into()))?;
    let version = update.version.clone();
    // download_and_install takes two callbacks that both report progress;
    // share the sink behind a mutex so both can call it.
    let progress = std::sync::Mutex::new(on_progress);
    update
        .download_and_install(
            |_received, total| {
                let total = total
                    .map(|t| format!("{:.1} MB", t as f64 / 1_048_576.0))
                    .unwrap_or_else(|| "?".into());
                crate::lock(&progress)(&format!("正在下载 v{version}（{total}）…"));
            },
            || crate::lock(&progress)("下载完成，正在安装并重启…"),
        )
        .await
        .map_err(|e| AppError::Update(format!("安装失败：{e}")))?;
    app.restart();
}
