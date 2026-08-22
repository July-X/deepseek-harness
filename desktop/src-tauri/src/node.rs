//! Locating and validating the Node.js runtime that runs the kernel.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::process::{quiet, run_with_progress};
use crate::settings::Settings;

/// The engine range dsh declares (`^22.19.0 || >=24.0.0`).
const MIN_COMPATIBLE: (u32, u32, u32) = (22, 19, 0);
const MAJOR_ALT_FLOOR: u32 = 24;

/// What the shell found out about a Node candidate.
#[derive(Debug, Clone, Serialize)]
pub struct NodeInfo {
    pub path: String,
    pub version: Option<String>,
    pub ok: bool,
    pub reason: String,
}

/// Parse `v22.19.0`-style output into (major, minor, patch).
fn parse_version(output: &str) -> Option<(u32, u32, u32)> {
    let text = output.trim().strip_prefix('v')?;
    let mut parts = text.split('.');
    // Without the explicit `parse::<u32>()` annotations the compiler reports
    // E0282 (`type annotations needed`) because each `parse()` is generic
    // over the target integer type and has no constraint on its own.
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts
        .next()?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse::<u32>()
        .ok()?;
    Some((major, minor, patch))
}

/// Whether a parsed version satisfies the dsh engine requirement.
fn compatible((major, minor, _patch): (u32, u32, u32)) -> bool {
    (major == MIN_COMPATIBLE.0 && minor >= MIN_COMPATIBLE.1) || major >= MAJOR_ALT_FLOOR
}

/// Ask a node executable for its version.
pub fn version_of(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    let output = quiet(&mut cmd).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.trim().to_string();
    (parse_version(&text).is_some()).then_some(version)
}

/// Probe a candidate node executable and report how usable it is.
pub fn probe(path: &Path) -> NodeInfo {
    let path = path.to_string_lossy().into_owned();
    if !fs::metadata(&path).map(|m| m.is_file()).unwrap_or(false) {
        return NodeInfo {
            path,
            version: None,
            ok: false,
            reason: "路径不存在或不可读".into(),
        };
    }
    match version_of(Path::new(&path)) {
        None => NodeInfo {
            path,
            version: None,
            ok: false,
            reason: "无法读取版本输出（可能不是有效的 node 可执行文件）".into(),
        },
        Some(version) => {
            let parsed = parse_version(&version).unwrap_or_default();
            if compatible(parsed) {
                NodeInfo {
                    path,
                    version: Some(version),
                    ok: true,
                    reason: "可用".into(),
                }
            } else {
                let (maj, min, pat) = parsed;
                NodeInfo {
                    path,
                    version: Some(version),
                    ok: false,
                    reason: format!("版本 {maj}.{min}.{pat} 不满足 dsh 要求（^22.19 || >=24）"),
                }
            }
        }
    }
}

/// Drop the trailing `.exe` for PATH lookup on Windows.
fn exe_name(name: &str) -> String {
    if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Candidate executable names probed in order when looking up a tool on
/// Windows. Windows PATH lookups honour `PATHEXT` (default
/// `.COM;.EXE;.BAT;.CMD;…`), and Node-adjacent tools are overwhelmingly
/// shipped as `.cmd` shims into the user-level npm prefix
/// (`%AppData%\npm\pnpm.cmd`) instead of `.exe`. Probing `.cmd` first
/// matches the layout every npm `install -g` produces, then falls through
/// to `.exe` (system-wide installs and `pnpm` standalone) and finally the
/// bare name (PATH entries that already include an extension). Outside
/// Windows only the bare name is valid.
#[cfg(windows)]
const WINDOWS_EXE_CANDIDATES: &[&str] = &[".cmd", ".exe", ""];

#[cfg(windows)]
fn which_in_dir(name: &str, dir: &Path) -> Option<PathBuf> {
    for ext in WINDOWS_EXE_CANDIDATES {
        let candidate = dir.join(format!("{name}{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(not(windows))]
fn which_in_dir(name: &str, dir: &Path) -> Option<PathBuf> {
    let candidate = dir.join(name);
    candidate.is_file().then_some(candidate)
}

/// Find `node` on the PATH.
fn from_path() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe_name("node"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Well-known install locations probed when `node` is not on the PATH.
fn common_locations() -> Vec<PathBuf> {
    let mut out = vec![
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ];
    if cfg!(windows) {
        out.push(PathBuf::from(r"C:\Program Files\nodejs\node.exe"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(Path::new(&local).join(r"Programs\nodejs\node.exe"));
        }
    }
    out
}

/// Resolve the node executable from explicit config, then the environment.
pub fn resolve(settings: &Settings) -> NodeInfo {
    if let Some(path) = settings.node_path.as_ref() {
        let info = probe(Path::new(path));
        if info.ok {
            return info;
        }
        // Fall through to detection; keep the reason so the UI can explain
        // why the configured path was rejected.
        if let Some(found) =
            from_path().or_else(|| common_locations().into_iter().find(|p| p.is_file()))
        {
            let mut detected = probe(&found);
            detected.reason = format!(
                "配置的路径不可用（{}），已自动回退到：{}",
                info.reason,
                found.display()
            );
            return detected;
        }
        return NodeInfo {
            path: path.clone(),
            ok: false,
            version: None,
            reason: format!("配置路径不可用，且环境中也未找到 node：{}", info.reason),
        };
    }
    let found = from_path().or_else(|| common_locations().into_iter().find(|p| p.is_file()));
    match found {
        Some(path) => probe(&path),
        None => NodeInfo {
            path: String::new(),
            version: None,
            ok: false,
            reason: "未检测到 Node.js。请安装 Node.js 22.19+（或 >=24）后重试，或在设置中手动指定 node 路径。".into(),
        },
    }
}

/// Find a usable pnpm executable for installing kernels.
///
/// Prefer an explicit config path, then the folder next to the resolved
/// `node`, then the PATH. pnpm is the installer for kernel versions; it is
/// not bundled with Node, so a missing pnpm surfaces as an install-time
/// error with setup guidance. The Windows probe tolerates both `.cmd`
/// shims (the npm-prefix layout) and standalone `.exe` installs.
pub fn resolve_pnpm(settings: &Settings, node_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = settings.pnpm_path.as_ref() {
        if Path::new(path).is_file() {
            return Some(PathBuf::from(path));
        }
    }
    if let Some(p) = which_in_dir("pnpm", node_dir) {
        return Some(p);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if let Some(p) = which_in_dir("pnpm", &dir) {
                return Some(p);
            }
        }
    }
    None
}

/// Find an npm executable that ships with the resolved node. npm is needed
/// only as a fallback installer when pnpm is missing; on the common layout
/// it sits next to `node.exe` / `node` and on PATH. An explicit
/// `settings.npm_path` wins when present (advanced users with a portable
/// npm), then the node-sibling and PATH searches use the same `.cmd` /
/// `.exe` / bare-name probe as pnpm.
pub fn find_npm(settings: &Settings, node_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = settings.npm_path.as_ref() {
        if Path::new(path).is_file() {
            return Some(PathBuf::from(path));
        }
    }
    if let Some(p) = which_in_dir("npm", node_dir) {
        return Some(p);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if let Some(p) = which_in_dir("npm", &dir) {
                return Some(p);
            }
        }
    }
    None
}

/// Resolve pnpm, installing it on demand when only Node is present.
///
/// The three-tier lookup (`settings.pnpm_path`, alongside `node`, PATH) is
/// tried first. When none of them hit and `npm` is reachable, run
/// `npm install -g pnpm` once, stream every line back through `on_progress`,
/// and re-run the lookup so the just-installed binary is returned. The full
/// npm transcript is written to `log_path` so the user can inspect failures
/// without rerunning the install.
pub fn ensure_pnpm(
    settings: &Settings,
    node_dir: &Path,
    log_path: &Path,
    mut on_progress: impl FnMut(&str),
) -> Result<PathBuf, String> {
    if let Some(p) = resolve_pnpm(settings, node_dir) {
        return Ok(p);
    }
    let npm = find_npm(settings, node_dir).ok_or_else(|| {
        "未检测到 pnpm，也未找到可用的 npm（无法自动安装）。请先安装 Node.js，再执行 `npm install -g pnpm`，或在「设置」中手动指定 pnpm 可执行文件路径。"
            .to_string()
    })?;
    on_progress("未检测到 pnpm，正在通过 npm 自动安装（首次需要联网，常见 30 秒~2 分钟）");
    let cwd = node_dir.to_path_buf();
    let status = run_with_progress(
        &npm,
        &["install", "-g", "pnpm"],
        &cwd,
        log_path,
        |line| on_progress(line),
    )
    .map_err(|e| {
        format!(
            "无法运行 npm 以自动安装 pnpm：{e}。请检查 Node.js 安装，或在「设置」中手动指定 pnpm 路径"
        )
    })?;
    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".into());
        return Err(format!(
            "自动安装 pnpm 失败（npm 退出码 {code}），常见原因：网络受限、企业代理未配置 npm registry、或权限不足（macOS/Linux 上默认 prefix 需写 /usr/local，建议改用 NVM 或 nvs）。完整日志：{log}\n\n也可手动执行 `npm install -g pnpm`，或在「设置」中指定已下载的 pnpm 路径。",
            log = log_path.display()
        ));
    }
    // `npm install -g` writes the new script into the npm prefix bin dir,
    // which on the common layout is already on PATH — re-running the
    // three-tier resolver picks the just-installed binary up. Falling back
    // to the explicitly-configured prefix handles the unusual case where
    // the user has a custom prefix that PATH does not see.
    if let Some(p) = resolve_pnpm(settings, node_dir) {
        on_progress("pnpm 已就绪");
        return Ok(p);
    }
    if let Ok(prefix) = npm_prefix(&npm, &cwd) {
        let candidate = prefix.join(if cfg!(windows) { "pnpm.cmd" } else { "pnpm" });
        if candidate.is_file() {
            on_progress("pnpm 已就绪");
            return Ok(candidate);
        }
    }
    Err(format!(
        "npm install -g pnpm 已完成但仍未在常见位置找到 pnpm 可执行文件。请检查 npm prefix 与 PATH 设置，或在「设置」中手动指定 pnpm 路径。完整日志：{}",
        log_path.display()
    ))
}

/// Ask npm where it would install global packages — the parent dir of the
/// script bin we are about to look in.
fn npm_prefix(npm: &Path, cwd: &Path) -> Result<PathBuf, String> {
    let mut cmd = Command::new(npm);
    cmd.args(["config", "get", "prefix"]).current_dir(cwd);
    let output = quiet(&mut cmd)
        .output()
        .map_err(|e| format!("无法读取 npm prefix：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "npm config get prefix 失败（退出码 {:?}）",
            output.status.code()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let prefix = text.trim();
    if prefix.is_empty() {
        return Err("npm prefix 为空".into());
    }
    Ok(PathBuf::from(prefix))
}
