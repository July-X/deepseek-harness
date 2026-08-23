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
            app.manage(AppState {
                data_dir,
                running: Mutex::new(None),
                node_cache: Mutex::new(None),
            });
            updater::spawn_background_check(app.handle());
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
