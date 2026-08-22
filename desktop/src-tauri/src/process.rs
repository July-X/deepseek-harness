//! Suppressing the console window Windows allocates for child processes,
//! plus a shared helper that streams long-running child output to both a
//! log file and an in-process progress callback.
//!
//! The shell is a GUI-subsystem app: every `Command` spawned without
//! `CREATE_NO_WINDOW` makes Windows briefly allocate a console window, which
//! the user sees as a flashing terminal. All helper-process spawns in this
//! crate go through `quiet`.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Hide the console window Windows would otherwise flash for the child.
/// No-op on other platforms.
pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Spawn a long-running child (`pnpm`, `npm`, …) and stream each stdout and
/// stderr line to both `log_path` and `on_progress`, returning once the
/// process exits.
///
/// `.cmd` files cannot be spawned directly on Windows, so they are routed
/// through the command shell there; everywhere else the executable is run
/// directly. Each output stream is drained on its own thread so a full OS
/// pipe buffer can never deadlock the other stream; the lines travel over a
/// channel back to this thread, which is the only caller of `on_progress`.
/// A heartbeat keeps the caller informed when the child stays silent for
/// tens of seconds while resolving the dependency graph or talking to the
/// npm registry.
pub fn run_with_progress(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    log_path: &Path,
    mut on_progress: impl FnMut(&str),
) -> io::Result<ExitStatus> {
    let mut log = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)?;

    let mut child = spawn(exe, args, cwd)?;
    let stdout = child.stdout.take().expect("child stdout was piped");
    let stderr = child.stderr.take().expect("child stderr was piped");

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

    const HEARTBEAT_SECS: u64 = 10;
    let started = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_secs(HEARTBEAT_SECS)) {
            Ok(line) => {
                on_progress(line.trim_end());
                let _ = writeln!(log, "{line}");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let secs = started.elapsed().as_secs();
                on_progress(&format!(
                    "… 子进程仍在运行（已进行 {secs} 秒）"
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = drain_stdout.join();
    let _ = drain_stderr.join();

    reap(child)
}

fn spawn(exe: &Path, args: &[&str], cwd: &Path) -> io::Result<Child> {
    // Pin every child we spawn (pnpm/npm and friends) at the shell's
    // configured npm registry so the mirror choice is enforceable even when
    // the user's global .npmrc points elsewhere or a project-local .npmrc
    // is missing. `npm_config_registry` is the env var pnpm and npm both
    // consult as the highest-priority source.
    let registry = crate::registry::npm_registry_base();
    // Tauri ships as a Windows GUI-subsystem app and inherits only the
    // system PATH on launch; the user PATH (where `npm` and `pnpm` shims
    // live after `npm install -g`) is dropped unless we re-stamp it.
    // `env::merged_path` reads `HKCU\Environment\Path` once and joins it
    // onto whatever the process already has.
    let path = crate::env::merged_path();
    #[cfg(windows)]
    {
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into());
        let mut cmd = Command::new(comspec);
        cmd.arg("/C").arg(exe).args(args);
        // GUI shells start with an arbitrary cwd; the child must inherit an
        // explicit one or it resolves the nearest package.json upward and
        // installs into the wrong directory.
        cmd.current_dir(cwd);
        cmd.env("PATH", path);
        cmd.env("npm_config_registry", registry);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        quiet(&mut cmd).spawn()
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(exe);
        cmd.args(args);
        cmd.current_dir(cwd);
        cmd.env("PATH", path);
        cmd.env("npm_config_registry", registry);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.spawn()
    }
}

fn reap(mut child: Child) -> io::Result<ExitStatus> {
    #[cfg(windows)]
    {
        // `ComSpec /C` makes cmd.exe the direct child and the real program
        // its grandchild; waiting on cmd only returns after the grandchild
        // has already exited, so a plain wait is the right thing here.
        child.wait()
    }
    #[cfg(not(windows))]
    {
        child.wait()
    }
}