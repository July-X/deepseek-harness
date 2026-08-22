//! Tauri commands backing the management UI.
//!
//! All commands operate against the shared [`AppState`] (data directory plus
//! the running kernel child) and the persisted `settings.json`. Long-running
//! work (kernel install) runs off the main thread and reports progress over a
//! `tauri::ipc::Channel`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri::{WebviewUrl, WebviewWindowBuilder};
use url::Url;

use crate::{kernel, node, plugins, releases, settings};

/// Shared shell state installed as a Tauri managed state.
pub struct AppState {
    pub data_dir: PathBuf,
    pub running: Mutex<Option<Child>>,
}

/// Everything the management UI needs on the first render.
#[derive(Serialize)]
pub struct StatusView {
    pub kernel: kernel::KernelStatus,
    pub node: node::NodeInfo,
    pub settings: settings::Settings,
    pub kernel_log: String,
}

/// Read a bounded tail of a text file for display.
fn read_tail(path: &Path, max_bytes: usize) -> String {
    let Ok(meta) = fs::metadata(path) else {
        return String::new();
    };
    let Ok(file) = fs::File::open(path) else {
        return String::new();
    };
    use std::io::{Read, Seek};
    let mut reader = file;
    let offset = meta.len().saturating_sub(max_bytes as u64);
    // `Vec::with_capacity` cannot infer its element type until something
    // pins it; without the annotation `reader.read_to_end(&mut buf)` later
    // in this function needs the explicit hint.
    let mut buf: Vec<u8> = Vec::with_capacity(max_bytes);
    if offset > 0 {
        let _ = reader.seek(std::io::SeekFrom::Start(offset));
    }
    let _ = reader.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn resolve_node_path(data_dir: &Path) -> Result<PathBuf, String> {
    let s = settings::load(data_dir);
    let info = node::resolve(&s);
    if !info.ok {
        return Err(info.reason);
    }
    Ok(PathBuf::from(info.path))
}

/// The web-app level error prefix the UI must not swallow.
fn app_err(app: &AppState, e: impl std::fmt::Display) -> String {
    format!("{e}（数据目录：{}）", app.data_dir.display())
}

// --- status ---------------------------------------------------------------

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> StatusView {
    let data_dir = state.data_dir.clone();
    let settings = settings::load(&data_dir);
    let kernel_status = kernel::status(&data_dir, &settings);
    let node_info = node::resolve(&settings);
    let kernel_log = read_tail(&kernel::logs_dir(&data_dir).join("kernel.log"), 8 * 1024);
    StatusView {
        kernel: kernel_status,
        node: node_info,
        settings,
        kernel_log,
    }
}

#[tauri::command]
pub fn detect_node(state: State<'_, AppState>) -> node::NodeInfo {
    // Detection ignores any configured path: it reports what the environment
    // has, so the UI can pre-fill the setting.
    let data_dir = state.data_dir.clone();
    let mut s = settings::load(&data_dir);
    s.node_path = None;
    node::resolve(&s)
}

#[tauri::command]
pub fn save_settings(
    state: State<'_, AppState>,
    settings: settings::Settings,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    settings::save(&data_dir, &settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_kernel_log(state: State<'_, AppState>) -> String {
    read_tail(
        &kernel::logs_dir(&state.data_dir).join("kernel.log"),
        16 * 1024,
    )
}

// --- releases --------------------------------------------------------------

/// Fetch the official kernel release list for the update menu.
#[tauri::command]
pub fn fetch_releases() -> Result<releases::ReleaseList, String> {
    releases::list_releases().map_err(|e| e.to_string())
}

pub fn promise_pnpm(data_dir: &Path) -> Result<(PathBuf, PathBuf, node::NodeInfo), String> {
    let s = settings::load(data_dir);
    let node_info = node::resolve(&s);
    if !node_info.ok {
        return Err(node_info.reason);
    }
    let node_dir = Path::new(&node_info.path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let pnpm = node::resolve_pnpm(&s, &node_dir).ok_or_else(|| {
        "未找到 pnpm（安装 Node.js 后执行 `npm install -g pnpm`，或在设置中指定 pnpm 路径）"
            .to_string()
    })?;
    Ok((PathBuf::from(node_info.path.clone()), pnpm, node_info))
}

// --- kernel install / switch / remove --------------------------------------

/// Install a pinned kernel version from npm, streaming progress events.
#[tauri::command]
pub async fn install_kernel(
    state: State<'_, AppState>,
    version: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    let (node_path, pnpm_exe, _node_info) = promise_pnpm(&data_dir)?;
    // Clone the values the closure needs so we still own `data_dir` and
    // `version` for the post-install `set_active` / auto-start steps.
    let dir_for_thread = data_dir.clone();
    let pnpm_ex = pnpm_exe;
    let version_for_thread = version.clone();
    let send = on_event.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        kernel::install_version(&dir_for_thread, &pnpm_ex, &version_for_thread, |msg| {
            let _ = send.send(msg.to_string());
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    result.map_err(|e| e.to_string())?;

    // First kernel installed becomes active automatically; later installs
    // leave the current active version untouched.
    if kernel::read_active(&data_dir).is_none() {
        kernel::set_active(&data_dir, &version).map_err(|e| e.to_string())?;
        let _ = on_event.send(format!("已切换到版本 {version}"));
    }
    if !kernel::port_open(settings::load(&data_dir).port) {
        let _ = on_event.send("正在启动内核…".to_string());
        match kernel::start_maybe(&data_dir, &node_path) {
            Ok(Some(child)) => {
                crate::lock(&state.running).replace(child);
                let _ = on_event.send("内核已启动".to_string());
            }
            Ok(None) => {}
            Err(e) => {
                let _ = on_event.send(format!("启动失败：{e}"));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn activate_version(state: State<'_, AppState>, version: String) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    // The switch takes effect on the next start; a running kernel keeps
    // serving until the user restarts it.
    kernel::set_active(&data_dir, &version).map_err(|e| e.to_string())?;
    // 重新接线插件到新活动内核（失败不阻断切换，原因进入插件卡片警告）
    let settings = settings::load(&data_dir);
    let _ = plugins::ensure_wiring_quiet(&data_dir, &settings);
    Ok(())
}

#[tauri::command]
pub fn remove_version(state: State<'_, AppState>, version: String) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    kernel::uninstall(&data_dir, &version).map_err(|e| app_err(&state, e))
}

// --- kernel lifecycle -------------------------------------------------------

/// Start the active kernel. Idempotent: if the port already answers, this is
/// a no-op so repeated clicks are harmless.
#[tauri::command]
pub fn start_kernel(state: State<'_, AppState>) -> Result<u16, String> {
    let data_dir = state.data_dir.clone();
    let node_path = resolve_node_path(&data_dir)?;
    // 启动前校正插件接线（跳过则内核可能不加载插件；失败不阻断启动）
    let settings = settings::load(&data_dir);
    let _ = plugins::ensure_wiring_quiet(&data_dir, &settings);
    if let Some(child) = kernel::start_maybe(&data_dir, &node_path).map_err(|e| e.to_string())? {
        crate::lock(&state.running).replace(child);
    }
    Ok(settings.port)
}

#[tauri::command]
pub fn stop_kernel(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = crate::lock(&state.running);
    if let Some(mut child) = guard.take() {
        kernel::stop(&mut child).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Open the harness web UI in a dedicated window.
///
/// The webview window is created on a fresh OS thread so the synchronous
/// command returns without holding the Tauri main thread. Webview creation
/// inside a synchronous command deadlocks on Windows (per
/// `WebviewWindowBuilder::new` docs), and even on macOS/Linux keeping the
/// main thread free for the eventual webview setup is the safer default.
#[tauri::command]
pub fn open_harness(app: AppHandle) -> Result<(), String> {
    let data_dir = crate::kernel::data_dir(&app);
    let settings = settings::load(&data_dir);
    if !kernel::port_open(settings.port) {
        return Err(format!(
            "内核未在运行（端口 {}），请先点击“启动内核”",
            settings.port
        ));
    }
    if let Some(existing) = app.get_webview_window("harness") {
        let _ = existing.set_focus();
        return Ok(());
    }
    let url =
        Url::parse(&format!("http://127.0.0.1:{}", settings.port)).map_err(|e| e.to_string())?;
    let handle = app.clone();
    std::thread::Builder::new()
        .name("dsh-open-harness".into())
        .spawn(move || {
            let result = WebviewWindowBuilder::new(&handle, "harness", WebviewUrl::External(url))
                .title("DeepSeek Harness 工作台")
                .inner_size(1280.0, 840.0)
                .build();
            if let Err(e) = result {
                eprintln!("dsh-desktop: failed to open harness window: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
// --- plugins ---------------------------------------------------------------

/// Snapshot of the plugin store and per-kernel materialization state.
#[tauri::command]
pub fn plugin_status(state: State<'_, AppState>) -> plugins::PluginStatus {
    let data_dir = state.data_dir.clone();
    let settings = settings::load(&data_dir);
    plugins::status(&data_dir, &settings)
}

/// Install a community plugin (npm package name or git URL) into the
/// central store, materialize it into every kernel, and wire the profile.
#[tauri::command]
pub async fn plugin_install(
    state: State<'_, AppState>,
    spec: String,
    mode: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    let settings = settings::load(&data_dir);
    let (_, pnpm_exe, _) = promise_pnpm(&data_dir)?;
    let send = on_event;
    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = |msg: &str| {
            let _ = send.send(msg.to_string());
        };
        plugins::install(&data_dir, &settings, &pnpm_exe, &spec, &mode, &mut progress)
    })
    .await
    .map_err(|e| e.to_string())?
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Fetch the latest version of one installed plugin and re-materialize.
#[tauri::command]
pub async fn plugin_update(
    state: State<'_, AppState>,
    id: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    let settings = settings::load(&data_dir);
    let (_, pnpm_exe, _) = promise_pnpm(&data_dir)?;
    let send = on_event;
    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = |msg: &str| {
            let _ = send.send(msg.to_string());
        };
        plugins::update(&data_dir, &settings, &pnpm_exe, &id, &mut progress)
    })
    .await
    .map_err(|e| e.to_string())?
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Uninstall a plugin everywhere (store, kernels, profile wiring).
#[tauri::command]
pub async fn plugin_uninstall(
    state: State<'_, AppState>,
    id: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    let settings = settings::load(&data_dir);
    let (_, pnpm_exe, _) = promise_pnpm(&data_dir)?;
    let send = on_event;
    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = |msg: &str| {
            let _ = send.send(msg.to_string());
        };
        plugins::uninstall(&data_dir, &settings, &pnpm_exe, &id, &mut progress)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// Re-materialize everything and re-wire the profile (「同步」button).
#[tauri::command]
pub async fn plugin_sync(
    state: State<'_, AppState>,
    on_event: Channel<String>,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    let settings = settings::load(&data_dir);
    let (_, pnpm_exe, _) = promise_pnpm(&data_dir)?;
    let send = on_event;
    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = |msg: &str| {
            let _ = send.send(msg.to_string());
        };
        plugins::sync_all(&data_dir, &settings, &pnpm_exe, &mut progress)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// Switch a plugin's materialization mode (link/copy) and re-sync.
#[tauri::command]
pub async fn plugin_set_mode(
    state: State<'_, AppState>,
    id: String,
    mode: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    let settings = settings::load(&data_dir);
    let (_, pnpm_exe, _) = promise_pnpm(&data_dir)?;
    let send = on_event;
    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = |msg: &str| {
            let _ = send.send(msg.to_string());
        };
        plugins::set_mode(&data_dir, &settings, &pnpm_exe, &id, &mode, &mut progress)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// Check every installed plugin against its origin for newer versions.
#[tauri::command]
pub async fn plugin_check_updates(
    state: State<'_, AppState>,
) -> Result<Vec<plugins::UpdateInfo>, String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || plugins::check_updates(&data_dir))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Search the community catalog (cached fetch of the reference market).
#[tauri::command]
pub async fn plugin_catalog(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<plugins::CatalogItem>, String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || plugins::catalog_search(&data_dir, &query))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
