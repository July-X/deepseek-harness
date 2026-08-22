//! Kernel lifecycle: installing pinned kernel versions, managing the active
//! Kernel lifecycle: installing pinned kernel versions, managing the active
//! version, and starting/stopping the `dsh web` process the shell embeds.
//!
//! Shell metadata lives under the harness home, next to the kernel's own
//! data: `<dsh_home>/desktop/` (`~/.dsh/desktop/` by default). A kernel
//! version is installed by running pnpm into a dedicated directory:
//!
//! ```text
//! <dsh_home>/desktop/kernels/<version>/
//!   package.json                     # minimal stub pnpm installs into
//!   node_modules/@deepseek-ai/dsh/   # the pinned kernel
//! ```
//!
//! The install uses the `hoisted` node-linker so `node_modules` stays flat —
//! the same layout npm produces — and the kernel entry point can be resolved
//! as a plain path without a symlink-capable filesystem. pnpm's global
//! content-addressable store makes repeat installs of other versions much
//! faster than a cold npm install. The `append-only` reporter emits one log
//! line per lifecycle event on stdout, which streams to the UI in real time.
//!
//! The active version is recorded in `<dsh_home>/desktop/active.txt`.

use std::fs;
use std::io;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use tauri::Manager;

use crate::error::AppError;
use crate::settings::{self, Settings};

/// dsh's own home directory name (see `@deepseek-ai/dsh-home-paths`).
pub const DSH_HOME_DIR_NAME: &str = ".dsh";
/// User-facing display form for the dsh home.
pub const DSH_HOME_LABEL: &str = "~/.dsh";
/// Shell metadata root under the dsh home: `<dsh_home>/desktop/`.
const SHELL_SUBDIR: &str = "desktop";
/// Relative path of the kernel's CLI entry inside an installed package.
const KERNEL_BIN_REL: &str = "node_modules/@deepseek-ai/dsh/lib/bin.js";

/// One installed kernel version on disk.
#[derive(Debug, Clone, Serialize)]
pub struct InstalledVersion {
    pub version: String,
    pub active: bool,
    pub size_bytes: u64,
}

/// Snapshot the UI renders on every status refresh.
#[derive(Debug, Clone, Serialize)]
pub struct KernelStatus {
    pub installed: Vec<InstalledVersion>,
    pub active: Option<String>,
    pub active_installed: bool,
    pub running: bool,
    pub port: u16,
    pub dsh_home: String,
    pub ever_installed: bool,
}

/// Shell metadata root: `<dsh_home>/desktop/`, where `<dsh_home>` is the
/// harness home (`~/.dsh` by default, overridable via `DSH_HOME`, mirroring
/// `@deepseek-ai/dsh-home-paths`). All shell state (kernels, settings, logs,
/// active pointer) lives under this one root next to the kernel's own data.
pub fn data_dir(app: &tauri::AppHandle) -> PathBuf {
    let home = std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(DSH_HOME_DIR_NAME));
    let dir = home.join(SHELL_SUBDIR);
    if fs::create_dir_all(&dir).is_ok() {
        return dir;
    }
    // Read-only dsh home: fall back to the OS app-data dir so the shell
    // still boots instead of failing at startup.
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// The user's OS home directory (`$HOME` on Unix, `%USERPROFILE%` on Windows).
fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn kernels_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("kernels")
}

pub fn kernel_dir(data_dir: &Path, version: &str) -> PathBuf {
    kernels_dir(data_dir).join(version)
}

pub fn active_file(data_dir: &Path) -> PathBuf {
    data_dir.join("active.txt")
}

pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Read the directory names that look like installed kernel versions.
pub fn list_installed(data_dir: &Path) -> Vec<InstalledVersion> {
    let dir = kernels_dir(data_dir);
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if !entry.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                continue;
            }
            let size = fs::metadata(kernel_dir(data_dir, &name).join(KERNEL_BIN_REL))
                .ok()
                .map(|m| m.len())
                .unwrap_or(0);
            out.push(InstalledVersion {
                version: name,
                active: false,
                size_bytes: size,
            });
        }
    }
    out
}

/// The currently active version, if any.
pub fn read_active(data_dir: &Path) -> Option<String> {
    fs::read_to_string(active_file(data_dir))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Record the active version. Persisted as plain text so the CLI tooling and
/// the app agree on a trivially inspectable format.
pub fn write_active(data_dir: &Path, version: Option<&str>) -> Result<(), AppError> {
    fs::create_dir_all(data_dir).map_err(|e| AppError::Io(e.to_string()))?;
    match version {
        Some(v) => fs::write(active_file(data_dir), format!("{v}\n")),
        None => fs::remove_file(active_file(data_dir)),
    }
    .map_err(|e| AppError::Io(e.to_string()))
}

/// Refresh the `active` flag on each installed version.
pub fn with_active(installed: &mut [InstalledVersion], active: Option<&str>) {
    for item in installed.iter_mut() {
        item.active = Some(item.version.as_str()) == active;
    }
}

/// Compose the full status snapshot.
pub fn status(data_dir: &Path, settings: &Settings) -> KernelStatus {
    let mut installed = list_installed(data_dir);
    let active = read_active(data_dir);
    with_active(&mut installed, active.as_deref());
    let active_installed = active
        .as_ref()
        .map(|v| kernel_dir(data_dir, v).join(KERNEL_BIN_REL).is_file())
        .unwrap_or(false);
    KernelStatus {
        ever_installed: !installed.is_empty(),
        installed,
        active,
        active_installed,
        running: port_open(settings.port),
        port: settings.port,
        dsh_home: DSH_HOME_LABEL.to_string(),
    }
}

/// Toggle which installed version `start` will run.
pub fn set_active(data_dir: &Path, version: &str) -> Result<(), AppError> {
    if !kernel_dir(data_dir, version).join(KERNEL_BIN_REL).is_file() {
        return Err(AppError::Kernel(format!(
            "版本 {version} 未安装或安装不完整"
        )));
    }
    write_active(data_dir, Some(version))
}

/// Delete an installed version. The caller must stop the kernel first when it
/// is the active version.
pub fn uninstall(data_dir: &Path, version: &str) -> Result<(), AppError> {
    if read_active(data_dir).as_deref() == Some(version) {
        return Err(AppError::Kernel(format!(
            "正在使用版本 {version}，请先停止并切换到其他版本"
        )));
    }
    let dir = kernel_dir(data_dir, version);
    if !dir.exists() {
        return Err(AppError::Kernel(format!("版本 {version} 未安装")));
    }
    fs::remove_dir_all(&dir).map_err(|e| AppError::Io(e.to_string()))
}

/// Ask pnpm to install `@deepseek-ai/dsh@<version>` into its directory.
///
/// `on_progress` receives human-readable stage messages plus every raw
/// installer log line, so the UI can show live output while the install runs.
pub fn install_version(
    data_dir: &Path,
    pnpm_exe: &Path,
    version: &str,
    mut on_progress: impl FnMut(&str),
) -> Result<(), AppError> {
    let dir = kernel_dir(data_dir, version);
    fs::create_dir_all(&dir).map_err(|e| AppError::Io(e.to_string()))?;
    let stub = dir.join("package.json");
    let stub_text = format!(
        "{{\"name\":\"dsh-kernel-{}\",\"private\":true,\"version\":\"1.0.0\"}}\n",
        version.replace('.', "_")
    );
    fs::write(&stub, stub_text).map_err(|e| AppError::Io(e.to_string()))?;

    let log_path = logs_dir(data_dir).join(format!("install-{version}.log"));
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::Io(e.to_string()))?;
    }

    on_progress("正在通过 pnpm 安装内核（首次通常需要 1~3 分钟，下方为实时日志）");
    let spec = format!("@deepseek-ai/dsh@{version}");
    let prefix = dir.to_str().unwrap_or_default();
    // `--ignore-workspace` keeps the install out of any workspace the user's
    // environment might expose; the kernel dir is a standalone package root.
    let args = [
        "add",
        "--prefix",
        prefix,
        "--ignore-workspace",
        "--config.node-linker=hoisted",
        "--reporter=append-only",
        spec.as_str(),
    ];
    let status = run_pnpm(pnpm_exe, &args, &log_path, &mut on_progress).map_err(|e| {
        AppError::Io(format!(
            "无法运行 pnpm（{e}）。请确认已安装 Node.js 与 pnpm"
        ))
    })?;
    on_progress("pnpm 已退出，正在校验安装结果");

    // pnpm ≥ 10 在存在被忽略的构建脚本（见 `pnpm approve-builds`）时会打印
    // `[ERR_PNPM_IGNORED_BUILDS]` 并以非零退出码结束，尽管安装产物已经就绪，
    // 退出码因此不能作为安装成功判据。以内核入口文件是否就位为准：
    // 退出码非零且产物缺失 → 失败；退出码非零且产物完整 → 降级为警告。
    let exit_code = status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "? (信号)".into());
    let bin_ready = dir.join(KERNEL_BIN_REL).is_file();
    if !status.success() && !bin_ready {
        return Err(AppError::Kernel(format!(
            "pnpm 安装失败（退出码 {exit_code}），请检查网络或 pnpm 配置后重试，详情见日志：{}",
            log_path.display()
        )));
    }
    if !bin_ready {
        return Err(AppError::Kernel(format!(
            "安装未产生预期的内核入口（{KERNEL_BIN_REL}），请查看日志：{}",
            log_path.display()
        )));
    }
    if !status.success() {
        on_progress(&format!(
            "注意：pnpm 以退出码 {exit_code} 结束（多为依赖构建脚本被忽略所致，可以在该内核目录运行 pnpm approve-builds 允许），内核文件已安装完成"
        ));
    }
    Ok(())
}

/// Spawn pnpm once with the given args, piping merged stdout+stderr line by
/// line to both `log_path` and `on_progress`.
///
/// `.cmd` files cannot be spawned directly on Windows, so they are routed
/// through the command shell there; everywhere else the pnpm script runs
/// directly. Each output stream is drained on its own thread so a full OS
/// pipe buffer can never deadlock the other stream; the lines travel over a
/// channel back to this thread, which is the only caller of `on_progress`.
pub(crate) fn run_pnpm(
    pnpm_exe: &Path,
    args: &[&str],
    log_path: &Path,
    mut on_progress: impl FnMut(&str),
) -> io::Result<std::process::ExitStatus> {
    use std::io::{BufRead, BufReader, Write};
    use std::sync::mpsc;

    let mut log = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)?;

    #[cfg(windows)]
    let mut child = {
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
        let mut cmd = Command::new(comspec);
        cmd.arg("/C").arg(pnpm_exe).args(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.spawn()?
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut cmd = Command::new(pnpm_exe);
        cmd.args(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.spawn()?
    };

    let stdout = child.stdout.take().expect("pnpm stdout was piped");
    let stderr = child.stderr.take().expect("pnpm stderr was piped");

    // One drain thread per stream; both feed the same channel. The receiver
    // loop below ends once both senders are dropped (streams closed).
    let (tx, rx) = mpsc::channel::<String>();
    let tx_err = tx.clone();
    let drain_stdout = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let drain_stderr = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if tx_err.send(line).is_err() {
                break;
            }
        }
    });

    for line in rx {
        on_progress(line.trim_end());
        let _ = writeln!(log, "{line}");
    }

    // The channel is closed by now, so both drains have finished their loops.
    let _ = drain_stdout.join();
    let _ = drain_stderr.join();

    child.wait()
}

/// Whether something is already listening on `127.0.0.1:port`.
pub fn port_open(port: u16) -> bool {
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
    let addr: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// The kernel log path for the current run.
fn run_log_path(data_dir: &Path) -> PathBuf {
    logs_dir(data_dir).join("kernel.log")
}

/// Spawn `dsh web --no-open` for the active version with output redirected to
/// the kernel log. On Unix the child is placed in its own process group so a
/// stop can reap the whole group.
pub fn start(data_dir: &Path, node: &Path, version: &str, port: u16) -> Result<Child, AppError> {
    let dir = kernel_dir(data_dir, version);
    let bin = dir.join(KERNEL_BIN_REL);
    if !bin.is_file() {
        return Err(AppError::Kernel(format!(
            "版本 {version} 未安装或安装不完整"
        )));
    }
    if port_open(port) {
        return Err(AppError::Kernel(format!(
            "端口 {port} 已被占用，可能已有内核在运行"
        )));
    }
    fs::create_dir_all(logs_dir(data_dir)).map_err(|e| AppError::Io(e.to_string()))?;
    let log = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(run_log_path(data_dir))
        .map_err(|e| AppError::Io(e.to_string()))?;
    let stdout = log.try_clone().map_err(|e| AppError::Io(e.to_string()))?;

    let mut cmd = Command::new(node);
    let port_arg: String = port.to_string();
    cmd.arg(&bin)
        .arg("web")
        .arg("--no-open")
        .arg("--port")
        .arg(port_arg)
        .current_dir(data_dir)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(log));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // Detach into a new session so `kill -pid` reaps the group.
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    cmd.spawn()
        .map_err(|e| AppError::Io(format!("无法启动内核：{e}")))
}

/// Start the active kernel unless something already listens on the port.
///
/// Returns `Ok(None)` when the port already answers (idempotent start), or
/// `Ok(Some(child))` when this call spawned the process.
pub fn start_maybe(data_dir: &Path, node: &Path) -> Result<Option<Child>, AppError> {
    let s = settings::load(data_dir);
    let port = s.port;
    if port_open(port) {
        return Ok(None);
    }
    let active = read_active(data_dir).ok_or_else(|| {
        AppError::Kernel("尚未选择内核版本，请先在“更新”页安装并切换到某一版本".into())
    })?;
    start(data_dir, node, &active, port).map(Some)
}

/// Stop a running kernel child, reaping its whole process group where the
/// platform supports it.
pub fn stop(child: &mut Child) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // Ask the group to terminate, then force-kill whatever survives.
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        let _ = child.wait();
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let _ = child.try_wait();
    }
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let _ = Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .status();
        let _ = child.wait();
    }
    Ok(())
}
