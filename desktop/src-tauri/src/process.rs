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
///
/// `extra_path_dirs` prepends the listed directories to the inherited
/// `PATH` on the child before it runs anything. macOS `.app` bundles launch
/// from a launchd environment whose `PATH` is just `/usr/bin:/bin:/usr/sbin:
/// /sbin`, so a user who installed Node and pnpm via Homebrew or nvm lives
/// outside that path; a child that invokes a Node-shebanged script
/// (`tsdown`, `tsc`, `node ./foo.js`, …) then dies with `env: node: No
/// such file or directory` even though the parent could find both binaries
/// to spawn them. Prepending `pnpm_exe.parent()` (and `node_dir` when the
/// caller has it) makes the child see the same `node` the parent used.
pub fn run_with_progress(
    exe: &Path,
    args: &[&str],
    cwd: &Path,
    log_path: &Path,
    extra_path_dirs: &[&Path],
    mut on_progress: impl FnMut(&str),
) -> io::Result<ExitStatus> {
    let mut log = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path)?;

    let mut child = spawn(exe, args, cwd, extra_path_dirs)?;
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
                on_progress(&format!("… 子进程仍在运行（已进行 {secs} 秒）"));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = drain_stdout.join();
    let _ = drain_stderr.join();

    reap(child)
}

fn spawn(exe: &Path, args: &[&str], cwd: &Path, extra_path_dirs: &[&Path]) -> io::Result<Child> {
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
    //
    // `extra_path_dirs` is layered on top of that: caller-supplied
    // directories (the validated `node` bin dir, `pnpm_exe.parent()` so
    // pnpm's own shim family is reachable, …) are prepended in order so
    // any Node-shebanged child can resolve `node` even on macOS .app
    // bundles, whose launchd PATH is system-only.
    let path = merge_extra_path(crate::env::merged_path(), extra_path_dirs);
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

/// Prepend `extra` entries to `base`. Empty / non-existent entries are
/// skipped so a caller that has no extra directories pays no cost. Path
/// separators follow the host (`;` on Windows, `:` elsewhere).
fn merge_extra_path(base: &str, extra: &[&Path]) -> String {
    if extra.is_empty() {
        return base.to_string();
    }
    #[cfg(windows)]
    const SEP: char = ';';
    #[cfg(not(windows))]
    const SEP: char = ':';
    let mut out = String::new();
    let mut first = true;
    for dir in extra {
        let Some(text) = dir.to_str() else { continue };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !first {
            out.push(SEP);
        }
        out.push_str(trimmed);
        first = false;
    }
    if !first {
        if !base.is_empty() {
            out.push(SEP);
            out.push_str(base);
        }
    } else {
        out.push_str(base);
    }
    out
}

fn reap(mut child: Child) -> io::Result<ExitStatus> {
    // On Windows `ComSpec /C` makes cmd.exe the direct child and the real
    // program its grandchild; waiting on cmd only returns after the
    // grandchild has already exited, so a plain wait is right everywhere.
    child.wait()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn merge_extra_path_no_extras_returns_base() {
        assert_eq!(merge_extra_path("/usr/bin:/bin", &[]), "/usr/bin:/bin");
    }

    #[test]
    fn merge_extra_path_prepends_single_dir() {
        let dir = PathBuf::from("/usr/local/bin");
        let merged = merge_extra_path("/usr/bin:/bin", &[dir.as_path()]);
        assert_eq!(merged, "/usr/local/bin:/usr/bin:/bin");
    }

    #[test]
    fn merge_extra_path_preserves_order() {
        let first = PathBuf::from("/opt/homebrew/bin");
        let second = PathBuf::from("/usr/local/bin");
        let merged = merge_extra_path("/usr/bin", &[first.as_path(), second.as_path()]);
        assert_eq!(merged, "/opt/homebrew/bin:/usr/local/bin:/usr/bin");
    }

    #[test]
    fn merge_extra_path_skips_empty_segments() {
        let empty = PathBuf::from("");
        let blank = PathBuf::from("   ");
        let real = PathBuf::from("/usr/local/bin");
        let merged =
            merge_extra_path("/usr/bin", &[empty.as_path(), blank.as_path(), real.as_path()]);
        assert_eq!(merged, "/usr/local/bin:/usr/bin");
    }

    #[test]
    fn merge_extra_path_empty_base_still_includes_extras() {
        let dir = PathBuf::from("/usr/local/bin");
        let merged = merge_extra_path("", &[dir.as_path()]);
        assert_eq!(merged, "/usr/local/bin");
    }

    #[test]
    fn merge_extra_path_all_extras_blank_falls_back_to_base() {
        // A slice of only whitespace/empty entries must leave the base
        // untouched — the helper should never panic on missing entries.
        let empty = PathBuf::from("");
        let merged = merge_extra_path("/usr/bin:/bin", &[empty.as_path()]);
        assert_eq!(merged, "/usr/bin:/bin");
    }
}
