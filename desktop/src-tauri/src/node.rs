//! Locating and validating the Node.js runtime that runs the kernel.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

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
    let output = Command::new(path).arg("--version").output().ok()?;
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
/// error with setup guidance.
pub fn resolve_pnpm(settings: &Settings, node_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = settings.pnpm_path.as_ref() {
        if Path::new(path).is_file() {
            return Some(PathBuf::from(path));
        }
    }
    let next_to_node = node_dir.join(if cfg!(windows) { "pnpm.cmd" } else { "pnpm" });
    if next_to_node.is_file() {
        return Some(next_to_node);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(exe_name("pnpm"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
