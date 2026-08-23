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

use crate::error::AppError;
use crate::{kernel, node, plugins, releases, settings, updater};

/// Shared shell state installed as a Tauri managed state.
pub struct AppState {
    pub data_dir: PathBuf,
    pub running: Mutex<Option<Child>>,
    /// Last resolved Node runtime, keyed by the configured node path. The
    /// status poll runs every few seconds; re-probing `node --version` each
    /// time would spawn a process per poll (slow on Windows, where process
    /// creation is expensive) for a result that only changes with the
    /// setting or the machine's Node install.
    pub node_cache: Mutex<Option<(Option<String>, node::NodeInfo)>>,
}

/// Everything the management UI needs on the first render.
#[derive(Serialize)]
pub struct StatusView {
    /// Version of the running shell itself (from tauri.conf.json).
    pub shell_version: String,
    pub kernel: kernel::KernelStatus,
    pub node: node::NodeInfo,
    pub settings: settings::Settings,
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

/// The web-app level error prefix the UI must not swallow.
fn app_err(data_dir: &Path, e: impl std::fmt::Display) -> String {
    format!("{e}（数据目录：{}）", data_dir.display())
}

// --- status ---------------------------------------------------------------

#[tauri::command]
pub async fn get_status(app: AppHandle, state: State<'_, AppState>) -> Result<StatusView, String> {
    let data_dir = state.data_dir.clone();
    // File probes and the port check run on a blocking worker: as a sync
    // command this poll would hold the Tauri main thread every few seconds.
    tauri::async_runtime::spawn_blocking(move || {
        let settings = settings::load(&data_dir);
        let kernel_status = kernel::status(&data_dir, &settings);
        let state = app.state::<AppState>();
        let node_info = cached_node(&state, &settings);
        StatusView {
            shell_version: app.package_info().version.to_string(),
            kernel: kernel_status,
            node: node_info,
            settings,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Resolve the Node runtime through the per-app cache; only a changed
/// `node_path` setting triggers a fresh probe.
fn cached_node(state: &AppState, settings: &settings::Settings) -> node::NodeInfo {
    let key = settings.node_path.clone();
    let mut guard = crate::lock(&state.node_cache);
    if let Some((cached_key, info)) = guard.as_ref() {
        if *cached_key == key {
            return info.clone();
        }
    }
    let info = node::resolve(settings);
    *guard = Some((key, info.clone()));
    info
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

/// One entry for the log-files modal tab list.
#[derive(Serialize)]
pub struct LogFileEntry {
    /// Just the basename (e.g. `kernel.log`, `install-0.1.0-rc.6.log`); the
    /// UI passes it back to `read_log_file`. Never expose absolute paths —
    /// the UI runs in a sandboxed webview and should not need them.
    pub name: String,
    /// File size in bytes; the modal shows it next to the tab name.
    pub size: u64,
}

/// List `*.log` files under the shell's log directory, newest first.
///
/// Files that disappear between `read_dir` and `metadata` are silently
/// skipped — install logs are rotated in place and may race with this scan.
#[tauri::command]
pub fn list_log_files(state: State<'_, AppState>) -> Vec<LogFileEntry> {
    let dir = kernel::logs_dir(&state.data_dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut out: Vec<LogFileEntry> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("log") {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Some(LogFileEntry { name, size })
        })
        .collect();

    // Newest first so the live `kernel.log` (touched on every status tick)
    // lands at index 0 — the modal's default tab.
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Read the tail of a named log file under the logs directory.
///
/// `name` must be a bare filename with no path separators; the function
/// refuses anything else to keep the UI's tab list from escaping the
/// logs directory. The same 16 KiB tail bound used by `get_kernel_log`
/// keeps the modal responsive on large install logs.
#[tauri::command]
pub fn read_log_file(state: State<'_, AppState>, name: String) -> Result<String, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!("非法的日志文件名：{name}"));
    }
    let path = kernel::logs_dir(&state.data_dir).join(&name);
    if !path.starts_with(kernel::logs_dir(&state.data_dir)) {
        return Err(format!("日志路径越界：{name}"));
    }
    Ok(read_tail(&path, 16 * 1024))
}

/// Reveal the shell's data directory in the OS file manager.
///
/// The path comes from `AppState.data_dir`, which `lib::setup` resolves
/// from `kernel::data_dir` and creates on first launch, so the directory
/// always exists at runtime. Going through the server side (instead of
/// letting the UI call `opener.open_path` directly) bypasses the opener
/// plugin's IPC scope check — `opener:default` only grants `open_url` /
/// `reveal_item_in_dir` / default URLs, not `open_path`. The `open` crate
/// that backs the plugin dispatches per-OS: `open` on macOS launches
/// Finder with the directory selected in its parent, `cmd /C start ""` on
/// Windows opens File Explorer on the directory itself.
#[tauri::command]
pub async fn open_data_dir(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = state.data_dir.clone();
    app.opener()
        .open_path(path.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| format!("无法打开数据目录：{e}"))
}

// --- shell self-update -----------------------------------------------------

/// Check GitHub for a newer shell release (manual「检查更新」button).
#[tauri::command]
pub async fn check_shell_update(app: AppHandle) -> Result<updater::ShellUpdateInfo, String> {
    updater::check(&app).await.map_err(|e| e.to_string())
}

/// Download, verify, and install the pending shell update, then restart.
#[tauri::command]
pub async fn install_shell_update(app: AppHandle, on_event: Channel<String>) -> Result<(), String> {
    updater::install(&app, move |line| {
        let _ = on_event.send(line.to_string());
    })
    .await
    .map_err(|e| e.to_string())
}

// --- releases --------------------------------------------------------------

/// Fetch the official kernel release list for the update menu.
#[tauri::command]
pub async fn fetch_releases() -> Result<releases::ReleaseList, String> {
    // ureq is synchronous; keep the blocking HTTPS fetch off the main thread.
    tauri::async_runtime::spawn_blocking(releases::list_releases)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Resolve pnpm against an already-probed node (the caller's cached
/// `node::NodeInfo`), auto-installing pnpm via npm when missing. Returns
/// (node_path, pnpm_exe).
pub fn promise_pnpm(
    data_dir: &Path,
    node_info: &node::NodeInfo,
    mut on_progress: impl FnMut(&str),
) -> Result<(PathBuf, PathBuf), String> {
    if !node_info.ok {
        return Err(node_info.reason.clone());
    }
    let s = settings::load(data_dir);
    let node_dir = Path::new(&node_info.path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    // The auto-install log lives under the shell's log dir next to the
    // install logs; rotation reuses the existing kernel::logs_dir helper.
    let pnpm_log = kernel::logs_dir(data_dir).join(format!(
        "pnpm-install-{}.log",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    let pnpm = node::ensure_pnpm(&s, &node_dir, &pnpm_log, &mut on_progress)?;
    Ok((PathBuf::from(node_info.path.clone()), pnpm))
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
    let settings = settings::load(&data_dir);
    let node_info = cached_node(&state, &settings);
    let (node_path, pnpm_exe) = promise_pnpm(&data_dir, &node_info, |msg| {
        let _ = on_event.send(msg.to_string());
    })?;
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
pub async fn activate_version(app: AppHandle, version: String) -> Result<(), String> {
    let data_dir = app.state::<AppState>().data_dir.clone();
    // Wiring runs pnpm against the store; keep the whole switch off the
    // main thread.
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        // The switch takes effect on the next start; a running kernel keeps
        // serving until the user restarts it.
        kernel::set_active(&data_dir, &version).map_err(|e| e.to_string())?;
        // 重新接线插件到新活动内核（失败不阻断切换，原因进入插件卡片警告）
        let settings = settings::load(&data_dir);
        let state = app.state::<AppState>();
        let node_info = cached_node(&state, &settings);
        let _ = plugins::ensure_wiring_quiet(&data_dir, &settings, &node_info);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn remove_version(state: State<'_, AppState>, version: String) -> Result<(), String> {
    let data_dir = state.data_dir.clone();
    // remove_dir_all on a kernel tree (node_modules included) can take
    // seconds on Windows; never on the main thread.
    tauri::async_runtime::spawn_blocking(move || {
        kernel::uninstall(&data_dir, &version).map_err(|e| app_err(&data_dir, e))
    })
    .await
    .map_err(|e| e.to_string())?
}

// --- kernel lifecycle -------------------------------------------------------

/// Start the active kernel. Idempotent: if the port already answers, this is
/// a no-op so repeated clicks are harmless.
#[tauri::command]
pub async fn start_kernel(app: AppHandle) -> Result<u16, String> {
    let data_dir = app.state::<AppState>().data_dir.clone();
    // Wiring and the child spawn both block (pnpm, process creation); run
    // them on a blocking worker rather than the Tauri main thread.
    tauri::async_runtime::spawn_blocking(move || -> Result<u16, String> {
        let settings = settings::load(&data_dir);
        let state = app.state::<AppState>();
        let node_info = cached_node(&state, &settings);
        if !node_info.ok {
            return Err(node_info.reason.clone());
        }
        let node_path = PathBuf::from(node_info.path.clone());
        // 启动前校正插件接线（跳过则内核可能不加载插件；失败不阻断启动）
        let _ = plugins::ensure_wiring_quiet(&data_dir, &settings, &node_info);
        if let Some(child) =
            kernel::start_maybe(&data_dir, &node_path).map_err(|e| e.to_string())?
        {
            kernel::write_pid(&data_dir, child.id());
            crate::lock(&state.running).replace(child);
        }
        Ok(settings.port)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stop the kernel and close the harness window, so the UI's「关闭工作台」
/// tears down the whole workbench rather than leaving a dead webview behind.
/// When the shell restarted since it spawned the kernel, the in-memory child
/// is gone but the pid file still names the process to reap.
///
/// The harness window is created with `closable(false)` (see `open_harness`),
/// so the OS title-bar close button is disabled and an accidental click on
/// it cannot drop the user's session. The deliberate path back through this
/// command still has to work, so the window goes through `destroy()` —
/// which forces the OS to close without honoring the closable flag — rather
/// than `close()`, which would be blocked by the same flag it set.
#[tauri::command]
pub async fn stop_kernel(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("harness") {
        let _ = window.destroy();
    }
    let data_dir = app.state::<AppState>().data_dir.clone();
    // kernel::stop waits for the child to exit (up to its kill timeout);
    // keep that wait off the main thread.
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let state = app.state::<AppState>();
        {
            let mut guard = crate::lock(&state.running);
            if let Some(mut child) = guard.take() {
                kernel::stop(&mut child).map_err(|e| e.to_string())?;
            }
        }
        let port = settings::load(&data_dir).port;
        if kernel::port_open(port) {
            // First try the pid file — the in-memory handle is gone
            // across a shell restart, but a previous shell wrote a
            // pid to <data_dir>/kernel.pid and the kernel it spawned
            // is still bound to this port. kill_pid already validates
            // that the pid still points at a dsh kernel before sending
            // signals, so a pid recycled to an unrelated process is
            // a no-op.
            let mut killed = false;
            if let Some(pid) = kernel::read_pid(&data_dir) {
                kernel::kill_pid(pid);
                killed = true;
            }
            // Fallback: when the dev/release shells run side-by-side
            // and the in-memory child + pid file are both missing
            // (e.g. start_maybe skipped the launch because the port
            // was already bound by the other shell's kernel), the
            // shell has no in-record way to find the listener. Walk
            // the listening port to recover its pid, then run it
            // through the same pid_is_kernel guard so a recycled
            // pid that happens to point at an unrelated process still
            // is left alone.
            if !killed {
                if let Some(pid) = kernel::port_listen_pid(port) {
                    #[cfg(unix)]
                    {
                        if kernel::pid_is_kernel(pid) {
                            kernel::kill_pid(pid);
                        }
                    }
                    // Windows has no cheap pid-is-kernel probe (a
                    // Get-CimInstance query is too slow for the stop
                    // path); kill_pid's taskkill matches the existing
                    // unguarded Windows stop behavior.
                    #[cfg(windows)]
                    kernel::kill_pid(pid);
                }
            }
        }
        kernel::clear_pid(&data_dir);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open the harness web UI in a dedicated window.
///
/// The webview window is created on a fresh OS thread so the synchronous
/// command returns without holding the Tauri main thread. Webview creation
/// inside a synchronous command deadlocks on Windows (per
/// `WebviewWindowBuilder::new` docs), and even on macOS/Linux keeping the
/// main thread free for the eventual webview setup is the safer default.
///
/// The window is created with `closable(false)` so the OS title-bar close
/// button is greyed out: an accidental click in the middle of a long task
/// would otherwise drop the user's session. The deliberate path back
/// through `stop_kernel` still works because that command uses `destroy()`
/// rather than `close()`, which forces the OS to honor the tear-down even
/// when the chrome close button is disabled. The Linux GTK+ backend is the
/// documented exception: it may not grey the button out for windows that
/// are already visible, so on Linux this is a behavioural hint rather than
/// a hard guarantee.
/// Open the dsh web workbench window. The native titlebar stays as
/// the standard macOS / Windows / Linux chrome rather than Overlay so
/// that the OS-level drag / resize / double-click-zoom continue to work
/// reliably (the WKWebView drag-region path through `start_dragging` IPC
/// is flaky under Tauri 2.11.5). The chrome-row pulse is owned by the
/// shell rather than the kernel's `packages/client/web/src/base.css`,
/// injected via `initialization_script(titlebar-pulse.js)`; the script
/// appends a `<style>` node with `!important` rules so the shell
/// override wins regardless of which kernel version is running and
/// regardless of load order between this script and the workbench's
/// own stylesheets.
#[tauri::command]
pub fn open_harness(app: AppHandle) -> Result<(), String> {
    let data_dir = crate::kernel::data_dir(&app);
    let settings = settings::load(&data_dir);
    if !kernel::port_open(settings.port) {
        return Err(format!(
            "内核未在运行（端口 {}），请先点击「启动工作台」",
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
                .closable(false)
                .initialization_script(include_str!("titlebar-pulse.js"))
                .build();
            if let Err(e) = result {
                eprintln!("dsh-desktop: failed to open harness window: {e}");
            }
            #[cfg(debug_assertions)]
            if let Ok(window) = app.get_webview_window("harness").ok_or("no harness window") {
                window.open_devtools();
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
// --- plugins ---------------------------------------------------------------

/// Snapshot of the plugin store and per-kernel materialization state.
#[tauri::command]
pub async fn plugin_status(state: State<'_, AppState>) -> Result<plugins::PluginStatus, String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings = settings::load(&data_dir);
        plugins::status(&data_dir, &settings)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Shared body of the plugin store commands: resolve pnpm against the
/// cached node probe (streaming any auto-install progress), then run the
/// `plugins::` operation on a blocking worker with progress forwarded over
/// the channel.
async fn run_plugin_command(
    app: AppHandle,
    on_event: Channel<String>,
    op: impl FnOnce(&Path, &settings::Settings, &Path, &mut dyn FnMut(&str)) -> Result<(), AppError>
        + Send
        + 'static,
) -> Result<(), String> {
    let data_dir = app.state::<AppState>().data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let settings = settings::load(&data_dir);
        let state = app.state::<AppState>();
        let node_info = cached_node(&state, &settings);
        let promise_send = on_event.clone();
        let (_, pnpm_exe) = promise_pnpm(&data_dir, &node_info, move |msg| {
            let _ = promise_send.send(msg.to_string());
        })?;
        let mut progress = |msg: &str| {
            let _ = on_event.send(msg.to_string());
        };
        op(&data_dir, &settings, &pnpm_exe, &mut progress).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Install a community plugin (npm package name or git URL) into the
/// central store, materialize it into every kernel, and wire the profile.
#[tauri::command]
pub async fn plugin_install(
    app: AppHandle,
    spec: String,
    mode: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    run_plugin_command(
        app,
        on_event,
        move |data_dir, settings, pnpm_exe, progress| {
            plugins::install(data_dir, settings, pnpm_exe, &spec, &mode, progress).map(|_| ())
        },
    )
    .await
}

/// Fetch the latest version of one installed plugin and re-materialize.
#[tauri::command]
pub async fn plugin_update(
    app: AppHandle,
    id: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    run_plugin_command(
        app,
        on_event,
        move |data_dir, settings, pnpm_exe, progress| {
            plugins::update(data_dir, settings, pnpm_exe, &id, progress).map(|_| ())
        },
    )
    .await
}

/// Uninstall a plugin everywhere (store, kernels, profile wiring).
#[tauri::command]
pub async fn plugin_uninstall(
    app: AppHandle,
    id: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    run_plugin_command(
        app,
        on_event,
        move |data_dir, settings, pnpm_exe, progress| {
            plugins::uninstall(data_dir, settings, pnpm_exe, &id, progress)
        },
    )
    .await
}

/// Re-materialize everything and re-wire the profile (「同步」button).
#[tauri::command]
pub async fn plugin_sync(app: AppHandle, on_event: Channel<String>) -> Result<(), String> {
    run_plugin_command(
        app,
        on_event,
        move |data_dir, settings, pnpm_exe, progress| {
            plugins::sync_all(data_dir, settings, pnpm_exe, progress)
        },
    )
    .await
}

/// Switch a plugin's materialization mode (link/copy) and re-sync.
#[tauri::command]
pub async fn plugin_set_mode(
    app: AppHandle,
    id: String,
    mode: String,
    on_event: Channel<String>,
) -> Result<(), String> {
    run_plugin_command(
        app,
        on_event,
        move |data_dir, settings, pnpm_exe, progress| {
            plugins::set_mode(data_dir, settings, pnpm_exe, &id, &mode, progress)
        },
    )
    .await
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

/// The full community catalog; search and filtering happen in the UI over
/// this cached list. `force` bypasses the cache window (「刷新目录」).
#[tauri::command]
pub async fn plugin_catalog(
    state: State<'_, AppState>,
    force: bool,
) -> Result<Vec<plugins::CatalogItem>, String> {
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || plugins::catalog(&data_dir, force))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
