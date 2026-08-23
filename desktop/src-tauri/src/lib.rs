//! dsh-desktop: a Tauri shell around the DeepSeek Harness kernel.
//!
//! The shell manages pinned kernel versions (installed via pnpm from the
//! official `dsh-v*` releases), runs the active kernel's `dsh web` server,
//! and opens its UI in a dedicated webview window. All management happens in
//! the local `ui/` frontend through the commands in [`commands`].

mod commands;
mod env;
mod error;
mod kernel;
mod node;
mod plugins;
mod process;
mod registry;
mod releases;
mod settings;
mod updater;
mod version;

use std::sync::Mutex;

use commands::AppState;
use tauri::Manager;

/// Lock a mutex, recovering the inner value when another thread panicked.
pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Application entry; invoked from `main.rs`.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = kernel::data_dir(app.handle());
            // Stamp the resolved data dir on stderr so a developer running
            // `tauri dev` alongside the installed release shell can see at
            // a glance which of the two (release → `~/.dsh/desktop/`, debug
            // → `~/.dsh/desktop-dev/`) actually owns this process. Cheap
            // insurance against the classic "I edited settings in the dev
            // shell and the release shell doesn't see it" foot-gun.
            eprintln!(
                "dsh-desktop: data_dir = {} (build: {})",
                data_dir.display(),
                if cfg!(debug_assertions) { "dev" } else { "release" }
            );
            // Reap orphaned dsh web kernels belonging to this data dir
            // BEFORE anything manages state: a crashed/killed shell leaves
            // its kernel running with cwd == data_dir, and two kernels on
            // the same project dir append to the same session log, which
            // corrupts it (seq gap). Must run before start_kernel can
            // observe "port already open" and report the orphan as a
            // healthy instance.
            kernel::reap_orphans(&data_dir);
            app.manage(AppState {
                data_dir,
                running: Mutex::new(None),
                node_cache: Mutex::new(None),
            });
            // Crash-recover any plugin-store staging dirs left behind by
            // an earlier shell run that died mid-update. The recovery is
            // a single read_dir scan on the happy path (no leftovers =
            // nothing to do) so it is safe to run unconditionally here
            // rather than gating on a marker file. Must run before any
            // plugin command touches the store, which it does not yet at
            // setup time.
            plugins::reconcile_store(&app.state::<AppState>().data_dir);
            updater::spawn_background_check(app.handle());
            // Auto-open DevTools on the management window in debug builds.
            // Tauri's webview keyboard shortcuts (`Cmd+Option+I`,
            // `Cmd+Shift+I`, F12) do not always reach WKWebView on macOS,
            // so the inspection surface has to be opened from the
            // embedding side. `setup` fires after the configured windows
            // are created, so the `main` webview is already retrievable
            // here; the gated `#[cfg(debug_assertions)]` keeps release
            // builds (which ship `with_devtools(false)` anyway) free of
            // the call.
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                window.open_devtools();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::detect_node,
            commands::save_settings,
            commands::get_kernel_log,
            commands::open_data_dir,
            commands::check_shell_update,
            commands::install_shell_update,
            commands::fetch_releases,
            commands::install_kernel,
            commands::activate_version,
            commands::remove_version,
            commands::start_kernel,
            commands::stop_kernel,
            commands::open_harness,
            commands::plugin_status,
            commands::plugin_install,
            commands::plugin_update,
            commands::plugin_uninstall,
            commands::plugin_sync,
            commands::plugin_set_mode,
            commands::plugin_check_updates,
            commands::plugin_catalog,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the dsh-desktop app");

    // Reap the kernel when the shell exits so no dsh web process is left
    // serving after the app quits. The in-memory child covers kernels this
    // session spawned; the pid file covers orphans left by an earlier shell
    // run (e.g. after a crash), guarded by `kill_pid`'s kernel check.
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = handle.try_state::<AppState>() {
                {
                    let mut guard = lock(&state.running);
                    if let Some(mut child) = guard.take() {
                        let _ = kernel::stop(&mut child);
                    }
                }
                let data_dir = state.data_dir.clone();
                if !kernel::port_open(settings::load(&data_dir).port) {
                    // Port free: either nothing runs or stop() above reaped
                    // it — drop a stale pid record so the next start is clean.
                    kernel::clear_pid(&data_dir);
                } else if let Some(pid) = kernel::read_pid(&data_dir) {
                    kernel::kill_pid(pid);
                    kernel::clear_pid(&data_dir);
                }
            }
        }
    });
}
