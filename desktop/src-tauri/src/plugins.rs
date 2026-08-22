//! Community plugin management: a central store under the harness home, per-kernel
//! materialization (link or copy), profile wiring, and update checks.
//!
//! All plugin sources live in <home>/plugins/ (the store), never inside a
//! kernel installation. Each installed kernel reads plugins from its own
//! <data_dir>/kernels/<version>/plugins/ directory, which the shell
//! materializes from the store either as a symlink (link mode, default) or a
//! real copy (copy mode). The active kernel's profile (profiles/<profile>/)
//! then declares each plugin as a dependency pointing at that materialized
//! directory plus a bundle layer when the plugin declares dsh.bundle,
//! mirroring what the kernel's plugin CLI produces, so switching kernels
//! never reinstalls anything - it only re-materializes and rewires.
//!
//! Design notes: docs/plugin-management.md in the desktop deliverable.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::process::quiet;
use crate::version::cmp_versions;
use crate::{commands, kernel, settings};

/// Default profile the shell wires plugins into (the kernel's web surface).
pub const DEFAULT_PROFILE: &str = "web";
/// Store directory name under the harness home.
const STORE_SUBDIR: &str = "plugins";
/// The shell's plugin inventory file inside the store directory.
const STORE_FILE: &str = "store.json";
/// Per-plugin fetch marker inside each store entry.
const SOURCE_MARKER: &str = ".dsh-source.json";
/// Community catalog, primary source: the dsh-plugin.org hub (the data feed
/// behind the DSH-Plugin Hub plugin center).
const HUB_CATALOG_URL: &str = "https://dsh-plugin.org/api/plugins.zh.json";
/// Community catalog, fallback source: the reference market's listing, used
/// when the hub is unreachable.
const MARKET_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/losebird/dsh-plugin-market/main/registry/all.json";
/// Catalog cache file under the shell data dir.
const CATALOG_CACHE_FILE: &str = "plugins-catalog.json";
/// Catalog cache freshness window.
const CATALOG_TTL_SECS: u64 = 6 * 3600;
/// Materialization metadata directory name inside kernel plugins dirs.
const META_SUBDIR: &str = ".meta";
/// Spec prefix for a pnpm link: (symlink) dependency.
const SPEC_LINK: &str = "link:";
/// Spec prefix for a pnpm file: (store copy) dependency.
const SPEC_FILE: &str = "file:";
/// Marker of shell-written dependency specs pointing at a kernel plugins dir.
const WIRED_MARK: &str = "desktop/kernels/";

const USER_AGENT: &str = concat!("dsh-desktop/", env!("CARGO_PKG_VERSION"));

// --- data model ------------------------------------------------------------

/// One installed plugin in the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StoreItem {
    /// Filesystem-safe plugin key (package/repo name with slashes replaced).
    pub id: String,
    /// Display name (npm package name or repo shorthand).
    pub name: String,
    /// Fetch origin: npm or git.
    pub origin: String,
    /// Fetch source: npm package name (optionally @version) or git URL (optionally #tag).
    pub source: String,
    pub installed_version: String,
    /// Latest known version, refreshed by check_updates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Desired materialization mode: link or copy.
    pub mode: String,
    /// Whether the source pins a version (npm @version / git #tag).
    pub pinned: bool,
    /// Seconds since epoch, for display.
    pub installed_at: String,
    /// Seconds since epoch of the last fetch, for display.
    pub updated_at: String,
    /// Human-facing repo URL for git-origin plugins.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Default for StoreItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            origin: String::from("npm"),
            source: String::new(),
            installed_version: String::new(),
            latest_version: None,
            mode: String::from("link"),
            pinned: false,
            installed_at: String::new(),
            updated_at: String::new(),
            repo_url: None,
            description: None,
        }
    }
}

/// The persisted store document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub items: Vec<StoreItem>,
    #[serde(rename = "lastCheckedAt", skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    /// Last wiring/install failure surfaced to the UI, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            schema_version: 1,
            items: Vec::new(),
            last_checked_at: None,
            warning: None,
        }
    }
}

/// Per-kernel materialization record, one JSON file per plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KernelMeta {
    /// Actual mode on disk: link or copy.
    mode: String,
    /// The store version this materialization reflects.
    version: String,
    synced_at: String,
}

/// One row the management UI renders.
#[derive(Debug, Clone, Serialize)]
pub struct PluginRow {
    pub id: String,
    pub name: String,
    pub origin: String,
    pub source: String,
    pub installed_version: String,
    pub latest_version: Option<String>,
    pub pinned: bool,
    /// Desired mode from the store.
    pub desired_mode: String,
    /// Actual mode in the active kernel, when materialized there.
    pub actual_mode: Option<String>,
    /// Whether the active kernel's materialization is present and current.
    pub synced: bool,
    /// Whether the active kernel's profile already loads this plugin.
    pub wired: bool,
    pub repo_url: Option<String>,
    pub description: Option<String>,
    pub installed_at: String,
    pub updated_at: String,
}

/// Aggregate plugin status for the management UI.
#[derive(Debug, Clone, Serialize)]
pub struct PluginStatus {
    pub rows: Vec<PluginRow>,
    pub profile: String,
    pub active_kernel: Option<String>,
    /// Number of plugins with a known newer version.
    pub updates: usize,
    pub last_checked_at: Option<String>,
    pub warning: Option<String>,
}

/// One catalog entry surfacing in the plugin center.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub description: String,
    pub stars: u64,
    #[serde(default)]
    pub forks: u64,
    pub downloads: u64,
    pub verified: bool,
    pub repo: Option<String>,
    /// Install spec: npm package name or git URL (with #tag when known).
    pub spec: String,
    /// npm or git, derived from the entry's install method.
    pub origin: String,
    pub category: String,
    /// Latest published version string (may carry a leading `v`).
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// ISO timestamp of the last upstream update, when known.
    #[serde(default)]
    pub updated: String,
    /// Human-facing detail page (dsh-plugin.org or the repository).
    #[serde(default)]
    pub detail_url: String,
}

/// npm registry document slice we need.
#[derive(Debug, Deserialize)]
struct NpmDoc {
    #[serde(rename = "dist-tags", default)]
    dist_tags: BTreeMap<String, String>,
    #[serde(default)]
    versions: BTreeMap<String, NpmVersionDoc>,
}

#[derive(Debug, Deserialize)]
struct NpmVersionDoc {
    #[serde(default)]
    dist: Option<NpmDist>,
}

#[derive(Debug, Deserialize)]
struct NpmDist {
    #[serde(default)]
    tarball: String,
}

/// A parsed install request.
#[derive(Debug, Clone)]
pub struct PluginSpec {
    pub origin: String,
    /// npm package name or git URL.
    pub source: String,
    /// Optional pinned version (npm semver) or tag (git).
    pub pin: Option<String>,
    /// Filesystem-safe store id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Human-facing repo URL for git origin.
    pub repo_url: Option<String>,
}

// --- paths ------------------------------------------------------------------

/// Central store root: <home>/plugins/, next to the profile dirs the store
/// feeds. data_dir is <home>/desktop/ (see kernel::data_dir).
pub fn store_dir(data_dir: &Path) -> PathBuf {
    data_dir
        .parent()
        .map(|home| home.join(STORE_SUBDIR))
        .unwrap_or_else(|| data_dir.join(STORE_SUBDIR))
}

fn store_file(data_dir: &Path) -> PathBuf {
    store_dir(data_dir).join(STORE_FILE)
}

fn store_plugin_dir(data_dir: &Path, id: &str) -> PathBuf {
    store_dir(data_dir).join(id)
}

fn kernel_plugins_dir(data_dir: &Path, version: &str) -> PathBuf {
    kernel::kernel_dir(data_dir, version).join("plugins")
}

fn kernel_plugin_dir(data_dir: &Path, version: &str, id: &str) -> PathBuf {
    kernel_plugins_dir(data_dir, version).join(id)
}

fn kernel_meta_file(data_dir: &Path, version: &str, id: &str) -> PathBuf {
    kernel_plugins_dir(data_dir, version)
        .join(META_SUBDIR)
        .join(format!("{id}.json"))
}

fn profile_dir(data_dir: &Path, profile: &str) -> PathBuf {
    data_dir
        .parent()
        .map(|home| home.join("profiles").join(profile))
        .unwrap_or_else(|| data_dir.join("profiles").join(profile))
}

fn wiring_log_path(data_dir: &Path) -> PathBuf {
    kernel::logs_dir(data_dir).join("plugin-wiring.log")
}

fn plugin_log_path(data_dir: &Path, id: &str) -> PathBuf {
    kernel::logs_dir(data_dir).join(format!("plugin-{id}.log"))
}

/// Map a package/repo name to a filesystem-safe store id. Path traversal is
/// structurally impossible afterwards: slashes become double underscores and
/// dot / empty segments are rejected outright.
pub fn id_for_name(raw: &str) -> Result<String, AppError> {
    let name = raw.trim();
    if name.is_empty() || name.len() > 200 {
        return Err(AppError::Plugin("插件名称为空或过长".into()));
    }
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(AppError::Plugin(format!(
                "非法的插件名称 {name:?}（包含空段或 ..）"
            )));
        }
    }
    Ok(name.replace('/', "__"))
}

fn now_epoch_secs() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default()
}

// --- store persistence ------------------------------------------------------

pub fn load_store(data_dir: &Path) -> Store {
    let Ok(text) = fs::read_to_string(store_file(data_dir)) else {
        return Store::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_store(data_dir: &Path, store: &Store) -> Result<(), AppError> {
    fs::create_dir_all(store_dir(data_dir)).map_err(|e| AppError::Io(e.to_string()))?;
    let text = serde_json::to_string_pretty(store).map_err(|e| AppError::Io(e.to_string()))?;
    fs::write(store_file(data_dir), text + "\n").map_err(|e| AppError::Io(e.to_string()))?;
    ensure_store_npmrc(data_dir)
}

/// Write a local .npmrc in the store directory.  Fresh pnpm defaults a
/// `minimumReleaseAge` of ~3 days so locked dev/rc versions stay installable
/// without waiting out the gate, and pins the registry mirror the desktop
/// shell already uses so mirror-only scoped packages resolve.  Replaces any
/// existing file so a previous (broken) shape gets corrected in place.
fn ensure_store_npmrc(data_dir: &Path) -> Result<(), AppError> {
    let npmrc = store_dir(data_dir).join(".npmrc");
    let registry = crate::registry::npm_registry_base();
    let text = format!(
        "minimumReleaseAge=0\nregistry={registry}\n@deepseek-ai:registry={registry}\n"
    );
    fs::write(&npmrc, text).map_err(|e| AppError::Io(e.to_string()))?;
    Ok(())
}

fn store_item(data_dir: &Path, id: &str) -> Option<StoreItem> {
    load_store(data_dir)
        .items
        .into_iter()
        .find(|item| item.id == id)
}

fn upsert_item(data_dir: &Path, item: StoreItem) -> Result<(), AppError> {
    let mut store = load_store(data_dir);
    if let Some(existing) = store.items.iter_mut().find(|i| i.id == item.id) {
        *existing = item;
    } else {
        store.items.push(item);
    }
    save_store(data_dir, &store)
}

fn remove_item(data_dir: &Path, id: &str) -> Result<(), AppError> {
    let mut store = load_store(data_dir);
    store.items.retain(|item| item.id != id);
    save_store(data_dir, &store)
}

fn read_meta(data_dir: &Path, version: &str, id: &str) -> Option<KernelMeta> {
    let text = fs::read_to_string(kernel_meta_file(data_dir, version, id)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_meta(data_dir: &Path, version: &str, id: &str, meta: &KernelMeta) -> Result<(), AppError> {
    if let Some(parent) = kernel_meta_file(data_dir, version, id).parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::Io(e.to_string()))?;
    }
    let text = serde_json::to_string(meta).map_err(|e| AppError::Io(e.to_string()))?;
    fs::write(kernel_meta_file(data_dir, version, id), text)
        .map_err(|e| AppError::Io(e.to_string()))
}

// --- spec parsing -----------------------------------------------------------

/// Split an npm spec into (name, optional pin). The last @ after the scope
/// prefix separates the version; @scope/name@1.2.3 parses as (@scope/name,
/// 1.2.3). Plain names pass through.
fn split_npm_spec(spec: &str) -> Result<(String, Option<String>), AppError> {
    let s = spec.trim();
    if s.starts_with('@') {
        let (head, rest) = s
            .split_once('/')
            .ok_or_else(|| AppError::Plugin(format!("非法的 npm 包名 {spec:?}")))?;
        let rest = rest.trim();
        let (name, pin) = match rest.rsplit_once('@') {
            Some((n, p)) if !n.is_empty() && !p.is_empty() && !p.contains('/') => {
                (n, Some(p.to_string()))
            }
            _ => (rest, None),
        };
        let name = format!("{head}/{name}");
        return Ok((name, pin));
    }
    match s.rsplit_once('@') {
        Some((n, p)) if !n.is_empty() && !p.is_empty() && !p.contains('/') => {
            Ok((n.to_string(), Some(p.to_string())))
        }
        _ => Ok((s.to_string(), None)),
    }
}

/// Parse an install request into a PluginSpec. Accepts npm package names
/// (with optional @version) and git URLs (https, git@, or owner/repo
/// shorthand, with optional #tag).
pub fn parse_spec(spec: &str) -> Result<PluginSpec, AppError> {
    let s = spec.trim().trim_end_matches('/');
    if s.is_empty() || s.len() > 500 {
        return Err(AppError::Plugin("安装地址为空或过长".into()));
    }
    if s.starts_with("git@") || s.contains("://") || s.contains("github.com/") {
        // git 来源：[url][#tag]
        let (url, pin) = match s.split_once('#') {
            Some((u, tag)) if !u.is_empty() && !tag.is_empty() => (u, Some(tag.to_string())),
            _ => (s, None),
        };
        let repo_url = s.contains("github.com/").then(|| url.to_string());
        // URL 含空路径段（协议双斜杠），先归一成 owner/repo 形状再映射 id
        let id_base = url
            .trim_start_matches("git@")
            .split("://")
            .last()
            .unwrap_or(url)
            .trim_end_matches(".git")
            .replace(':', "/");
        let id = id_for_name(&id_base)?;
        let name = url
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or(url)
            .to_string();
        return Ok(PluginSpec {
            origin: "git".into(),
            source: url.to_string(),
            pin,
            id,
            name,
            repo_url,
        });
    }
    // owner/repo 简写：非 npm 样式（不含 @ 且含斜杠）按 GitHub 仓库处理
    if s.contains('/') && !s.starts_with('@') {
        let github = format!("https://github.com/{s}.git");
        let id = id_for_name(s)?;
        return Ok(PluginSpec {
            origin: "git".into(),
            source: github,
            pin: None,
            id,
            name: s.rsplit('/').next().unwrap_or(s).to_string(),
            repo_url: Some(format!("https://github.com/{s}")),
        });
    }
    // npm 来源
    let (name, pin) = split_npm_spec(s)?;
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-._@/".contains(c))
    {
        return Err(AppError::Plugin(format!("非法的 npm 包名 {spec:?}")));
    }
    let id = id_for_name(&name)?;
    Ok(PluginSpec {
        origin: "npm".into(),
        source: name.clone(),
        pin,
        id,
        name,
        repo_url: None,
    })
}

// --- version comparison -----------------------------------------------------
// Shared with the kernel release list: crate::version::cmp_versions.

/// Highest version among tag candidates, or None.
fn latest_tag<'a>(tags: impl Iterator<Item = &'a str>) -> Option<String> {
    tags.filter_map(|t| {
        let stripped = t.strip_prefix('v').unwrap_or(t);
        let head = stripped.split_once('-').map(|(h, _)| h).unwrap_or(stripped);
        let parts: Vec<&str> = head.split('.').collect();
        (parts.len() >= 2 && parts[..2].iter().all(|seg| seg.parse::<u64>().is_ok()))
            .then(|| t.to_string())
    })
    .max_by(|a, b| cmp_versions(a, b))
}

/// Whether a stored version string looks like semver (e.g. `v0.15.0`,
/// `1.2.3-rc.1`) rather than a git short hash (e.g. `v646c91c`).
///
/// Used by `is_newer_than` to detect the rare fallback path where an
/// unpinned git-origin repo has no usable semver tags: in that case
/// `installed_version` is the cloned HEAD short hash, and `cmp_versions`
/// would rank any semver tag ahead of it purely on numeric-segment
/// count. Filtering on shape first lets `is_newer_than` choose the
/// right comparison instead of trusting that ordering.
fn looks_like_semver(version: &str) -> bool {
    let stripped = version.strip_prefix('v').unwrap_or(version);
    let head = stripped.split_once('-').map(|(h, _)| h).unwrap_or(stripped);
    let parts: Vec<&str> = head.split('.').collect();
    parts.len() >= 2 && parts[..2].iter().all(|seg| seg.parse::<u64>().is_ok())
}

/// Whether the candidate `latest` is newer than the currently installed
/// `installed` for a plugin of the given origin.
///
/// - npm / pinned git: rank by `cmp_versions` against a semver baseline.
/// - unpinned git with a tag-shaped installed version (the common case
///   after `fetch_git` resolves the highest semver tag): same semver
///   rank.
/// - unpinned git with a hash-shaped installed version (the fallback
///   path for repos without any semver tags): `cmp_versions` would
///   rank the remote's tag-shaped `latest` ahead purely on numeric
///   segment count, so fall back to string equality — but only when
///   `latest` is also a hash. A tag against a hash means the remote
///   has no commit-graph signal to compare against, so report no
///   update until the user manually re-installs.
fn is_newer_than(latest: &str, installed: &str, origin: &str, pinned: bool) -> bool {
    if origin == "git" && !pinned && !looks_like_semver(installed) {
        if looks_like_semver(latest) {
            false
        } else {
            latest != installed
        }
    } else {
        cmp_versions(latest, installed) == Ordering::Greater
    }
}

// --- fetching ---------------------------------------------------------------

/// Run one command, collecting stdout for quick helpers (git ls-remote).
fn run_capture(program: &str, args: &[&str]) -> io::Result<(bool, String)> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    let output = quiet(&mut cmd).output()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok((output.status.success(), text))
}

/// Fetch the npm registry document for a package.
fn fetch_npm_doc(name: &str) -> Result<NpmDoc, String> {
    let url = format!("{}{}", crate::registry::npm_registry_base(), name);
    let mut response = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e: ureq::Error| e.to_string())?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e: ureq::Error| e.to_string())?;
    serde_json::from_str(&body).map_err(|e: serde_json::Error| e.to_string())
}

/// Download a tarball into a byte vector.
fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e: ureq::Error| e.to_string())?;
    response
        .body_mut()
        .read_to_vec()
        .map_err(|e: ureq::Error| e.to_string())
}

/// Extract a tgz into dest, stripping the leading package/ segment. Uses the
/// system tar (bsdtar on macOS/Windows, GNU tar elsewhere).
fn extract_tarball(tarball: &Path, dest: &Path) -> Result<(), String> {
    let mut cmd = Command::new("tar");
    let status = quiet(&mut cmd)
        .arg("-xzf")
        .arg(tarball)
        .arg("--strip-components=1")
        .arg("-C")
        .arg(dest)
        .status()
        .map_err(|e| format!("无法运行系统 tar：{e}"))?;
    if !status.success() {
        return Err(format!("tar 解包失败（退出码 {:?}）", status.code()));
    }
    Ok(())
}

fn write_source_marker(spec: &PluginSpec, version: &str, dest: &Path) -> Result<(), AppError> {
    let marker = serde_json::json!({
        "id": spec.id,
        "origin": spec.origin,
        "source": spec.source,
        "version": version,
        "fetchedAt": now_epoch_secs(),
    });
    let text = serde_json::to_string_pretty(&marker).map_err(|e| AppError::Io(e.to_string()))?;
    fs::write(dest.join(SOURCE_MARKER), text + "\n").map_err(|e| AppError::Io(e.to_string()))
}

/// Fetch a plugin into the store under a fresh tmp dir, then atomically swap
/// it into place. Returns the new store item, inheriting mode and latest.
/// `pnpm_exe` builds git-sourced plugins whose committed tree lacks `lib/`.
fn fetch_into_store(
    data_dir: &Path,
    pnpm_exe: &Path,
    spec: &PluginSpec,
    on_progress: &mut dyn FnMut(&str),
) -> Result<StoreItem, AppError> {
    let store = store_dir(data_dir);
    fs::create_dir_all(&store).map_err(|e| AppError::Io(e.to_string()))?;
    let tmp = store.join(format!(".tmp-{}-{}", spec.id, now_epoch_secs()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| AppError::Io(e.to_string()))?;

    let version = match spec.origin.as_str() {
        "npm" => fetch_npm(spec, &tmp, on_progress),
        "git" => fetch_git(spec, &tmp, pnpm_exe, on_progress),
        other => Err(AppError::Plugin(format!("未知来源 {other:?}"))),
    };
    let version = match version {
        Ok(v) => v,
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };

    on_progress("正在校验插件是否符合 dsh 规范");
    if let Err(e) = validate_plugin(&tmp) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }

    let final_dir = store_plugin_dir(data_dir, &spec.id);
    if final_dir.exists() {
        // 更新路径：旧目录先移走再删除，rename 失败也不破坏新内容
        let old = store.join(format!(".old-{}-{}", spec.id, now_epoch_secs()));
        let _ = fs::remove_dir_all(&old);
        fs::rename(&final_dir, &old).map_err(|e| AppError::Io(e.to_string()))?;
        let _ = fs::remove_dir_all(&old);
    }
    fs::rename(&tmp, &final_dir).map_err(|e| AppError::Io(e.to_string()))?;
    write_source_marker(spec, &version, &final_dir)?;

    let now = now_epoch_secs();
    let existing = store_item(data_dir, &spec.id);
    Ok(StoreItem {
        id: spec.id.clone(),
        name: spec.name.clone(),
        origin: spec.origin.clone(),
        source: spec.source.clone(),
        installed_version: version,
        latest_version: existing.as_ref().and_then(|e| e.latest_version.clone()),
        mode: existing
            .as_ref()
            .map(|e| e.mode.clone())
            .unwrap_or_else(|| String::from("link")),
        pinned: spec.pin.is_some(),
        installed_at: existing
            .as_ref()
            .map(|e| e.installed_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
        repo_url: spec.repo_url.clone(),
        description: None,
    })
}

fn fetch_npm(
    spec: &PluginSpec,
    dest: &Path,
    on_progress: &mut dyn FnMut(&str),
) -> Result<String, AppError> {
    on_progress(&format!("正在查询 npm registry：{}", spec.source));
    let doc =
        fetch_npm_doc(&spec.source).map_err(|e| AppError::Plugin(format!("查询 npm 失败：{e}")))?;
    let version = spec
        .pin
        .clone()
        .unwrap_or_else(|| doc.dist_tags.get("latest").cloned().unwrap_or_default());
    if version.is_empty() {
        return Err(AppError::Plugin(format!(
            "npm 上找不到包 {} 或其 latest 标记",
            spec.source
        )));
    }
    let tarball = doc
        .versions
        .get(&version)
        .and_then(|v| v.dist.as_ref())
        .map(|d| d.tarball.clone())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            AppError::Plugin(format!(
                "npm 上 {}@{version} 没有可下载的 tarball",
                spec.source
            ))
        })?;
    on_progress(&format!("正在下载 {}@{version} …", spec.source));
    let bytes = fetch_bytes(&tarball).map_err(|e| AppError::Plugin(format!("下载失败：{e}")))?;
    let tgz = dest.join(".pkg.tgz");
    fs::write(&tgz, bytes).map_err(|e| AppError::Io(e.to_string()))?;
    extract_tarball(&tgz, &dest.join("package"))
        .map_err(|e| AppError::Plugin(format!("解包失败：{e}（请确认系统存在 tar）")))?;
    let _ = fs::remove_file(&tgz);
    let _ = fs::remove_dir_all(dest.join("package"));
    Ok(version)
}

fn fetch_git(
    spec: &PluginSpec,
    dest: &Path,
    pnpm_exe: &Path,
    on_progress: &mut dyn FnMut(&str),
) -> Result<String, AppError> {
    let mut probe = Command::new("git");
    probe.arg("--version");
    if quiet(&mut probe).output().is_err() {
        return Err(AppError::Plugin(
            "未找到 git（git 来源的插件需要 git；请先安装 git）".into(),
        ));
    }

    // Resolve what to check out.
    // - pinned: the spec supplies `#tag`; use it directly.
    // - unpinned: pick the highest semver tag the remote has published,
    //   so the installed_version stored on disk is the same kind of
    //   string `check_updates` will compare against. A repo without any
    //   semver tag falls back to the default branch (HEAD short hash);
    //   `is_newer_than` handles that fallback specially so a fresh tag
    //   does not look "newer" than the hash on segment count alone.
    let branch = match spec.pin.as_ref() {
        Some(tag) => Some(tag.clone()),
        None => match git_latest_tag(&spec.source) {
            Ok(Some(tag)) => Some(tag),
            Ok(None) => None,
            Err(e) => {
                return Err(AppError::Plugin(format!("查询最新 tag 失败：{e}")));
            }
        },
    };

    on_progress(&format!("正在克隆 {}", spec.source));
    let mut cmd = Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(tag) = &branch {
        cmd.arg("--branch").arg(tag);
    }
    let status = quiet(&mut cmd)
        .arg(&spec.source)
        .arg(dest)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| AppError::Io(format!("无法运行 git：{e}")))?;
    if !status.success() {
        return Err(AppError::Plugin(format!(
            "git clone 失败（退出码 {:?}），请检查地址与网络",
            status.code()
        )));
    }
    build_git_plugin(dest, pnpm_exe, on_progress)?;
    if let Some(tag) = branch {
        return Ok(tag);
    }
    // Unpinned repo without any semver tags: cloned the default branch;
    // record the HEAD hash so the source marker still names something
    // stable and the user can see what commit they have.
    let dest_str = dest.to_str().unwrap_or("");
    let (ok, out) = run_capture("git", &["-C", dest_str, "rev-parse", "--short", "HEAD"])
        .map_err(|e| AppError::Io(e.to_string()))?;
    Ok(if ok {
        out.trim().to_string()
    } else {
        String::from("head")
    })
}

/// Build a git-sourced plugin right after cloning.
///
/// Git repos carry their build output in `.gitignore` (`lib/` is never
/// committed), so the freshly cloned tree cannot satisfy the loader until it
/// is built. The package's own `prepare` script is the npm-sanctioned hook
/// for exactly this; running it via pnpm keeps toolchain resolution inside
/// the plugin. Best-effort: when no `prepare` exists the plugin must ship
/// prebuilt output, and `validate_plugin` still guards the final state, so
/// this only reports failure when a declared prepare actually fails.
fn build_git_plugin(
    dest: &Path,
    pnpm_exe: &Path,
    on_progress: &mut dyn FnMut(&str),
) -> Result<(), AppError> {
    let root = match read_plugin_manifest(dest) {
        Ok(root) => root,
        Err(_) => return Ok(()), // validate_plugin reports the real problem
    };
    let has_prepare = root
        .get("scripts")
        .and_then(|s| s.get("prepare"))
        .and_then(|p| p.as_str())
        .map(|p| !p.is_empty())
        .unwrap_or(false);
    let main = root.get("main").and_then(|m| m.as_str()).unwrap_or("");
    let entry_ready = !main.is_empty()
        && (dest.join(main).is_file() || dest.join(format!("{main}.js")).is_file());
    // Prebuilt repo: nothing to do. Declared-but-unbuilt is the common case.
    if !has_prepare || entry_ready {
        return Ok(());
    }
    // Dependencies are required for the build script to find its tools
    // (tsdown etc.). install_store_deps runs later in link mode, but that is
    // too late — the entry check happens first, and copy mode skips it.
    on_progress("正在安装插件依赖并构建（pnpm，git 来源需要生成 lib/）");
    let log_path = dest.join(".dsh-build.log");
    let args = [
        "install",
        "--ignore-workspace",
        "--config.node-linker=hoisted",
        "--reporter=append-only",
        // pnpm 11+ refuses to silently skip a transitive dependency's
        // build script: `ERR_PNPM_IGNORED_BUILDS` turns into a non-zero
        // exit code even when the parent `prepare` ran fine and emitted
        // `lib/`. Plugins commonly pull in something like `node-pty`
        // whose native compile we don't actually need here — the
        // project's own `tsdown` step has already produced the entry.
        "--config.strict-dep-builds=false",
        "--config.enable-pre-post-scripts=true",
    ];
    // pnpm runs the package's `prepare` lifecycle script automatically after
    // install when enable-pre-post-scripts is on.
    let status =
        kernel::run_pnpm(pnpm_exe, &args, dest, &log_path, &mut *on_progress).map_err(|e| {
            AppError::Io(format!(
                "无法运行 pnpm（{e}）。请确认已安装 Node.js 与 pnpm"
            ))
        })?;
    if !status.success() {
        return Err(AppError::Plugin(format!(
            "插件构建失败（退出码 {:?}）：`prepare` 未成功生成入口。详情见 {}",
            status.code(),
            log_path.display()
        )));
    }
    Ok(())
}

fn read_plugin_manifest(plugin_root: &Path) -> Result<serde_json::Value, serde_json::Error> {
    let text =
        fs::read_to_string(plugin_root.join("package.json")).map_err(serde_json::Error::io)?;
    serde_json::from_str(&text)
}

/// Whether the plugin directory declares runtime dependencies.
fn manifest_has_deps(plugin_root: &Path) -> bool {
    let Ok(root) = read_plugin_manifest(plugin_root) else {
        return false;
    };
    root.get("dependencies")
        .and_then(|d| d.as_object())
        .map(|d| !d.is_empty())
        .unwrap_or(false)
}

/// Whether the plugin declares a bundle layer.
fn manifest_is_bundle(plugin_root: &Path) -> bool {
    let Ok(root) = read_plugin_manifest(plugin_root) else {
        return false;
    };
    root.get("dsh")
        .and_then(|d| d.get("bundle"))
        .and_then(|b| b.get("patch"))
        .and_then(|p| p.as_str())
        .map(|p| !p.is_empty())
        .unwrap_or(false)
}

/// What the kernel needs to load an installed plugin: a parseable
/// package.json with a name; when the package declares a bundle layer its
/// patch file must exist, and regardless of bundling it needs a resolvable
/// `main`/`exports` entry. Runs right after fetch so a non-conforming
/// package fails the install loudly instead of breaking the next kernel boot.
///
/// The entry check must run even when a bundle layer is present: plugins
/// commonly declare both (the bundle patches the client UI while `main`
/// loads the server half). Returning early on the bundle branch let git
/// source installs through without their build output (`lib/` is
/// gitignored), which then crashed the kernel at ESM resolution time.
fn validate_plugin(dir: &Path) -> Result<(), AppError> {
    let root = read_plugin_manifest(dir)
        .map_err(|_| AppError::Plugin("不符合 dsh 插件规范：缺少可解析的 package.json".into()))?;
    let name = root.get("name").and_then(|n| n.as_str()).unwrap_or("");
    if name.is_empty() {
        return Err(AppError::Plugin(
            "不符合 dsh 插件规范：package.json 缺少 name 字段".into(),
        ));
    }
    if let Some(patch) = root
        .get("dsh")
        .and_then(|d| d.get("bundle"))
        .and_then(|b| b.get("patch"))
        .and_then(|p| p.as_str())
    {
        if patch.is_empty() || !dir.join(patch).is_file() {
            return Err(AppError::Plugin(format!(
                "不符合 dsh 插件规范：声明了 bundle 层但包内找不到 patch 文件 {patch:?}，内核启动将无法加载该层"
            )));
        }
        // Fall through: the runtime entry below is still required.
    }
    let has_exports = root.get("exports").is_some();
    if !has_exports {
        let main = root.get("main").and_then(|m| m.as_str()).unwrap_or("");
        if main.is_empty() {
            return Err(AppError::Plugin(
                "不符合 dsh 插件规范：既未声明 dsh.bundle.patch，也没有 main/exports 入口，内核无法加载"
                    .into(),
            ));
        }
        // Node 解析 main 时允许省略 .js 后缀，两者都接受。
        if dir.join(main).is_file() || dir.join(format!("{main}.js")).is_file() {
            return Ok(());
        }
        return Err(AppError::Plugin(format!(
            "不符合 dsh 插件规范：main 入口 {main:?} 在包内不存在，内核无法加载。git 来源的插件通常需要在包内执行一次构建（如 `pnpm run prepare`）生成 lib/"
        )));
    }
    Ok(())
}

/// Install the plugin's own dependencies inside the store dir. Only link
/// mode needs this (copy mode lets the profile's pnpm handle them).
fn install_store_deps(
    data_dir: &Path,
    pnpm_exe: &Path,
    id: &str,
    on_progress: &mut dyn FnMut(&str),
) -> Result<(), AppError> {
    let dir = store_plugin_dir(data_dir, id);
    let log_path = plugin_log_path(data_dir, id);
    if !manifest_has_deps(&dir) {
        return Ok(());
    }
    on_progress("正在安装插件自身依赖（pnpm）");

    // Delete any stale lockfile so pnpm re-resolves without minimumReleaseAge violations
    // from entries locked to recently-published rc versions.  A fresh install without a
    // lockfile re-resolves everything from the registry and is always safe.
    let lockfile = dir.join("pnpm-lock.yaml");
    if lockfile.is_file() {
        fs::remove_file(&lockfile).ok();
    }

    let args = [
        "install",
        "--ignore-workspace",
        "--config.node-linker=hoisted",
        "--reporter=append-only",
        // See `build_git_plugin`: pnpm 11+'s default ignored-builds
        // accounting turns into exit code 1 here too, even when the
        // node_modules tree is fine.
        "--config.strict-dep-builds=false",
    ];
    let status =
        kernel::run_pnpm(pnpm_exe, &args, &dir, &log_path, &mut *on_progress).map_err(|e| {
            AppError::Io(format!(
                "无法运行 pnpm（{e}）。请确认已安装 Node.js 与 pnpm"
            ))
        })?;
    if !status.success() && !dir.join("node_modules").is_dir() {
        return Err(AppError::Plugin(format!(
            "插件依赖安装失败（退出码 {:?}），详情见日志：{}",
            status.code(),
            log_path.display()
        )));
    }
    if !status.success() {
        on_progress(
            "注意：pnpm 以非零退出码结束（多为依赖构建脚本被忽略所致），插件依赖已基本就绪",
        );
    }
    Ok(())
}

// --- materialization --------------------------------------------------------

/// Materialize one plugin into one kernel: link (symlink, junction on
/// Windows) or copy, recorded in .meta/<id>.json. Returns the actual mode.
pub fn materialize_one(
    data_dir: &Path,
    version: &str,
    item: &StoreItem,
) -> Result<String, AppError> {
    let source = store_plugin_dir(data_dir, &item.id);
    let target = kernel_plugin_dir(data_dir, version, &item.id);
    let meta = read_meta(data_dir, version, &item.id);

    // Resolve the store path once: if the store source itself is a symlink
    // (e.g. a git-origin plugin cloned into the store), use the actual
    // filesystem location so the kernel plugin dir gets a direct link —
    // avoiding the double-symlink chain that breaks Node's realpath.
    let resolved_source = fs::symlink_metadata(&source)
        .ok()
        .filter(|m| m.file_type().is_symlink())
        .and_then(|_| fs::read_link(&source).ok())
        .unwrap_or_else(|| source.to_path_buf());

    let fresh = meta
        .as_ref()
        .map(|m| m.version == item.installed_version && m.mode == item.mode)
        .unwrap_or(false);

    // If the metadata says nothing changed AND the target exists, verify the
    // target symlink is actually correct.  A prior run may have left a stale
    // double-symlink chain even though the recorded version and mode are
    // unchanged — falling through re-creates the correct direct link.
    if fresh && target.exists() {
        let target_ok = fs::symlink_metadata(&target)
            .ok()
            .filter(|m| m.file_type().is_symlink())
            .and_then(|_| fs::read_link(&target).ok())
            .map(|link| link == resolved_source)
            .unwrap_or(false);
        if target_ok {
            return Ok(meta.map(|m| m.mode).unwrap_or_else(|| item.mode.clone()));
        }
    }

    // 清除旧产物（错误残留：非链接目录、指向别处的链接或旧版本副本）
    remove_materialized(data_dir, version, &item.id);

    let mut actual = item.mode.clone();
    if item.mode == "link" && make_dir_link(&resolved_source, &target).is_err() {
        // 链接失败（Windows 权限、文件系统不支持）→ 降级复制
        actual = String::from("copy");
        eprintln!(
            "dsh-desktop: link failed for {}; falling back to copy",
            item.id
        );
    }
    if actual == "copy" {
        copy_tree(&source, &target)
            .map_err(|e| AppError::Io(format!("复制插件到内核失败：{e}")))?;
    }
    if !target.exists() {
        return Err(AppError::Plugin(format!(
            "物化失败：{} 在内核 {version} 中未就绪",
            item.id
        )));
    }
    write_meta(
        data_dir,
        version,
        &item.id,
        &KernelMeta {
            mode: actual.clone(),
            version: item.installed_version.clone(),
            synced_at: now_epoch_secs(),
        },
    )?;
    Ok(actual)
}

/// Remove a plugin's materialization from one kernel (link or copy residue).
fn remove_materialized(data_dir: &Path, version: &str, id: &str) {
    let target = kernel_plugin_dir(data_dir, version, id);
    match fs::symlink_metadata(&target) {
        Ok(md) if md.file_type().is_symlink() => {
            let _ = fs::remove_file(&target);
        }
        Ok(_) => {
            let _ = fs::remove_dir_all(&target);
        }
        Err(_) => {}
    }
    let _ = fs::remove_file(kernel_meta_file(data_dir, version, id));
}

#[cfg(unix)]
fn make_dir_link(source: &Path, target: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn make_dir_link(source: &Path, target: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(source, target)
}

/// Recursively copy source into target, replacing whatever exists.
fn copy_tree(source: &Path, target: &Path) -> io::Result<()> {
    if target.is_symlink() {
        let _ = fs::remove_file(target);
    } else if target.exists() {
        let _ = fs::remove_dir_all(target);
    }
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            let _ = fs::remove_file(&to);
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Materialize a plugin into every installed kernel.
pub fn sync_kernels(data_dir: &Path, item: &StoreItem) -> Result<(), AppError> {
    for version in kernel::list_installed(data_dir) {
        materialize_one(data_dir, &version.version, item)?;
    }
    Ok(())
}

// --- profile wiring ---------------------------------------------------------

/// Relative path from from_dir to to (both under the same root), or the
/// absolute path when they share no common prefix.
fn relative_path(from_dir: &Path, to: &Path) -> PathBuf {
    let to_path = to;
    let from: Vec<Component> = from_dir.components().collect();
    let to: Vec<Component> = to_path.components().collect();
    let mut common = 0;
    while common < from.len() && common < to.len() && from[common] == to[common] {
        common += 1;
    }
    if common == 0 {
        return to_path.to_path_buf();
    }
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for part in &to[common..] {
        out.push(part.as_os_str());
    }
    out
}

/// Forward-slash path string for a dependency spec in package.json.
fn spec_path_string(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

/// Template bundles for a freshly initialized profile, mirroring the
/// kernel's profile templates.
fn template_bundles(profile: &str) -> Vec<String> {
    match profile {
        "web" => vec![
            String::from("@deepseek-ai/dsh-base"),
            String::from("@deepseek-ai/dsh-web-app"),
        ],
        "headless" => vec![
            String::from("@deepseek-ai/dsh-base"),
            String::from("@deepseek-ai/dsh-headless"),
        ],
        _ => vec![String::from("@deepseek-ai/dsh-base")],
    }
}

/// Read a profile manifest as a mutable JSON tree (round-trips unknown
/// fields). None when the profile directory is not initialized.
fn read_profile_json(
    data_dir: &Path,
    profile: &str,
) -> Result<Option<serde_json::Value>, AppError> {
    let path = profile_dir(data_dir, profile).join("package.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| AppError::Io(e.to_string()))
}

fn write_profile_json(
    data_dir: &Path,
    profile: &str,
    root: &serde_json::Value,
) -> Result<(), AppError> {
    let path = profile_dir(data_dir, profile).join("package.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::Io(e.to_string()))?;
    }
    let text = serde_json::to_string_pretty(root).map_err(|e| AppError::Io(e.to_string()))?;
    fs::write(path, text + "\n").map_err(|e| AppError::Io(e.to_string()))
}

/// Initialize a profile manifest the same way the kernel would, but with the
/// template bundle list baked in so wiring can precede the first boot.
fn ensure_profile(data_dir: &Path, profile: &str) -> Result<(), AppError> {
    let dir = profile_dir(data_dir, profile);
    let manifest_path = dir.join("package.json");
    if fs::metadata(&manifest_path).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(&dir).map_err(|e| AppError::Io(e.to_string()))?;
    let root = serde_json::json!({
        "name": format!("dsh-profile-{profile}"),
        "private": true,
        "dependencies": {},
        "dsh": { "profile": { "bundles": template_bundles(profile) } }
    });
    write_profile_json(data_dir, profile, &root)?;
    let patch = dir.join("cordis.patch.yml");
    if !patch.exists() {
        let _ = fs::write(&patch, "# Your patch layer for this dsh profile.\n[]\n");
    }
    let workspace = dir.join("pnpm-workspace.yaml");
    let needs_workspace = !workspace.exists()
        || fs::read_to_string(&workspace)
            .map(|t| !t.contains("minimumReleaseAge: 0"))
            .unwrap_or(false);
    if needs_workspace {
        let _ = fs::write(
            &workspace,
            "packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\nminimumReleaseAge: 0\n",
        );
    }
    Ok(())
}

/// Whether a profile dependency spec is one the shell wrote (points at a
/// kernel plugins dir). Protects CLI-managed dependencies from pruning.
fn is_managed_spec(spec: &str) -> bool {
    (spec.starts_with(SPEC_LINK) || spec.starts_with(SPEC_FILE)) && spec.contains(WIRED_MARK)
}

/// Reconcile the profile manifest against the store for the ACTIVE kernel:
/// set each item's dependency to the materialized dir, maintain bundle
/// layers, rewrite specs when the active kernel changed. Runs pnpm install
/// when the manifest changed or the profile's node_modules is missing.
///
/// The manifest write is transactional: when pnpm fails the manifest is
/// rolled back, because a bundles entry that cannot resolve crashes the
/// kernel at boot. An empty store still reconciles, so uninstalling the last
/// plugin prunes its residue instead of leaving an unresolvable layer behind.
///
/// Returns (wired_count, changed).
pub fn ensure_wiring(
    data_dir: &Path,
    settings: &settings::Settings,
    pnpm_exe: &Path,
    on_progress: &mut dyn FnMut(&str),
) -> Result<(usize, bool), AppError> {
    let store = load_store(data_dir);
    ensure_profile(data_dir, &settings.profile)?;

    // 物化活动内核，再据插件清单决定 bundle 层；没有活动内核且仍有插件时
    // 等内核装好再接线（store 为空则继续，让下面的清退逻辑跑掉残留）。
    let mut specs: BTreeMap<String, (String, bool)> = BTreeMap::new();
    match kernel::read_active(data_dir) {
        Some(active) => {
            for item in &store.items {
                refresh_store_peers(data_dir, item, &active)?;
                let actual = materialize_one(data_dir, &active, item)?;
                let prefix = if actual == "copy" {
                    SPEC_FILE
                } else {
                    SPEC_LINK
                };
                let rel = relative_path(
                    &profile_dir(data_dir, &settings.profile),
                    &kernel_plugin_dir(data_dir, &active, &item.id),
                );
                specs.insert(
                    item.name.clone(),
                    (
                        format!("{prefix}{}", spec_path_string(&rel)),
                        manifest_is_bundle(&kernel_plugin_dir(data_dir, &active, &item.id)),
                    ),
                );
            }
        }
        None if !store.items.is_empty() => return Ok((0, false)),
        None => {}
    }

    let mut root = read_profile_json(data_dir, &settings.profile)?
        .ok_or_else(|| AppError::Plugin("profile 尚未初始化".into()))?;
    let previous = root.clone();
    let changed = wire_manifest(&mut root, &specs, &settings.profile)?;

    // manifest 没变但 node_modules 缺失（上次 pnpm 失败或目录被清）也必须
    // 重装，否则 bundles 里的层解析不了，内核启动即崩。
    let profile = profile_dir(data_dir, &settings.profile);
    let node_modules_missing = !profile.join("node_modules").is_dir();
    if !changed && !node_modules_missing {
        return Ok((specs.len(), false));
    }
    if changed {
        write_profile_json(data_dir, &settings.profile, &root)?;
    }
    on_progress("正在同步 profile 依赖（pnpm install）");
    let log_path = wiring_log_path(data_dir);
    let status = kernel::run_pnpm(
        pnpm_exe,
        // See `build_git_plugin`: pnpm 11+'s default ignored-builds
        // accounting turns into exit code 1 even when resolution is
        // healthy. The profile's install only needs a usable
        // node_modules, which the existing fallback already tolerates
        // when wiring is unchanged, so silence the false positive here.
        &[
            "install",
            "--reporter=append-only",
            "--config.strict-dep-builds=false",
        ],
        &profile,
        &log_path,
        on_progress,
    )
    .map_err(|e| AppError::Io(format!("无法运行 pnpm（{e}）")))?;
    if !status.success() {
        if changed {
            let _ = write_profile_json(data_dir, &settings.profile, &previous);
        }
        return Err(AppError::Plugin(format!(
            "pnpm install 在 profile 中失败（退出码 {:?}），已回滚 profile 配置，详情见日志：{}",
            status.code(),
            log_path.display()
        )));
    }
    Ok((specs.len(), changed))
}

/// Quiet wiring for sync commands (kernel switch / start): failures are
/// recorded in the store for plugin_status.warning instead of blocking the
/// action.
pub fn ensure_wiring_quiet(data_dir: &Path, settings: &settings::Settings) -> Result<(), String> {
    let (_, pnpm_exe, _) = commands::promise_pnpm(data_dir, |_| {})?;
    let mut noop = |_: &str| {};
    match ensure_wiring(data_dir, settings, &pnpm_exe, &mut noop) {
        Ok(_) => {
            let mut store = load_store(data_dir);
            store.warning = None;
            let _ = save_store(data_dir, &store);
            Ok(())
        }
        Err(e) => {
            let mut store = load_store(data_dir);
            store.warning = Some(e.to_string());
            let _ = save_store(data_dir, &store);
            Err(e.to_string())
        }
    }
}

// --- update checks ----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub id: String,
    pub latest: Option<String>,
    pub error: Option<String>,
}

/// Check every store item against its origin's latest version. Stores the
/// results back into the store for the UI badge.
pub fn check_updates(data_dir: &Path) -> Result<Vec<UpdateInfo>, AppError> {
    let mut store = load_store(data_dir);
    let mut out = Vec::new();
    for item in &mut store.items {
        let (latest, error) = match item.origin.as_str() {
            "npm" => match fetch_npm_doc(&item.source) {
                Ok(doc) => (doc.dist_tags.get("latest").cloned(), None),
                Err(e) => (None, Some(e)),
            },
            "git" => match git_latest(item) {
                Ok(v) => (v, None),
                Err(e) => (None, Some(e)),
            },
            _ => (None, None),
        };
        let newer =
            latest.filter(|v| is_newer_than(v, &item.installed_version, &item.origin, item.pinned));
        item.latest_version = newer.clone();
        out.push(UpdateInfo {
            id: item.id.clone(),
            latest: newer,
            error,
        });
    }
    store.last_checked_at = Some(now_epoch_secs());
    save_store(data_dir, &store)?;
    Ok(out)
}

/// Latest version of a git-origin plugin: the highest semver tag the
/// remote has published. `fetch_git` aligns `installed_version` with
/// the same shape (a tag, or the HEAD hash as a fallback), so
/// `is_newer_than` can compare them directly.
///
/// The unpinned branch tracks the highest tag rather than the branch
/// HEAD — a developer who pushed new commits but has not cut a release
/// yet will not look "newer" than the user's last install. Plugin
/// authors publish releases via tags; that is what the user wants
/// notified about.
fn git_latest(item: &StoreItem) -> Result<Option<String>, String> {
    git_latest_tag(&item.source)
}

/// Highest semver tag the remote has published, used by `fetch_git`
/// (to pick the branch when the source is unpinned) and by `git_latest`
/// (to compare against the installed version). Returns None when the
/// remote has no usable tags.
fn git_latest_tag(source: &str) -> Result<Option<String>, String> {
    let (ok, out) =
        run_capture("git", &["ls-remote", "--tags", source]).map_err(|e| e.to_string())?;
    if !ok {
        return Ok(None);
    }
    let tags: Vec<String> = out
        .lines()
        .filter_map(|line| {
            let (_, ref_part) = line.split_once('\t')?;
            let tag = ref_part.strip_prefix("refs/tags/")?.trim_end_matches("^{}");
            Some(tag.to_string())
        })
        .collect();
    Ok(latest_tag(tags.iter().map(|s| s.as_str())))
}

// --- catalog ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CatalogDoc {
    #[serde(default)]
    items: Vec<CatalogRaw>,
}

#[derive(Debug, Deserialize)]
struct CatalogRaw {
    id: String,
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    stars: u64,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    verified: bool,
    #[serde(rename = "install", default)]
    install: Option<CatalogInstall>,
    #[serde(default)]
    category: String,
}

#[derive(Debug, Deserialize)]
struct CatalogInstall {
    #[serde(default)]
    method: String,
}

/// dsh-plugin.org short-key payload (`/api/plugins.zh.json`).
#[derive(Debug, Deserialize)]
struct HubRaw {
    /// Plugin slug.
    #[serde(default)]
    s: String,
    /// Owner slug (GitHub owner, lowercase).
    #[serde(default)]
    o: String,
    /// Display name.
    #[serde(default)]
    n: String,
    /// Latest version, e.g. `v3.22.1`.
    #[serde(default)]
    vr: String,
    /// Category id (interface/session/memory/tools/agent/workflow/...).
    #[serde(default)]
    c: String,
    /// Tags.
    #[serde(default)]
    t: Vec<String>,
    /// Description.
    #[serde(default)]
    d: String,
    /// GitHub repo `owner/name`.
    #[serde(default)]
    r: String,
    /// Verification state; `verified` means manually reviewed.
    #[serde(default)]
    v: String,
    /// Last upstream update (ISO 8601).
    #[serde(default)]
    u: String,
    /// Stars.
    #[serde(default)]
    sg: u64,
    /// Forks.
    #[serde(default)]
    fk: u64,
}

impl HubRaw {
    /// Normalize a hub entry to the shared catalog item. The hub's official
    /// install path is git (`dsh plugin add github:owner/repo`), so entries
    /// with a repo install from git; repo-less entries fall back to npm.
    fn into_item(self) -> CatalogItem {
        let repo = (!self.r.is_empty()).then_some(self.r.clone());
        let detail_url = if !self.o.is_empty() && !self.s.is_empty() {
            format!("https://dsh-plugin.org/zh/plugins/{}/{}", self.o, self.s)
        } else {
            repo.as_ref()
                .map(|r| format!("https://github.com/{r}"))
                .unwrap_or_default()
        };
        let (origin, spec) = match &repo {
            Some(r) => ("git", format!("https://github.com/{r}.git")),
            None => ("npm", self.n.clone()),
        };
        let id = if self.s.is_empty() {
            self.n.clone()
        } else {
            self.s.clone()
        };
        CatalogItem {
            id,
            name: self.n,
            kind: String::new(),
            description: self.d,
            stars: self.sg,
            forks: self.fk,
            downloads: 0,
            verified: self.v == "verified",
            repo,
            spec,
            origin: origin.to_string(),
            category: self.c,
            version: self.vr,
            tags: self.t,
            updated: self.u,
            detail_url,
        }
    }
}

/// Normalize a reference-market entry to the shared catalog item.
fn from_market_raw(raw: CatalogRaw) -> CatalogItem {
    let npm_origin = raw.package.is_some()
        || raw
            .install
            .as_ref()
            .map(|i| matches!(i.method.as_str(), "npm" | "pnpm" | "dsh-plugin-add"))
            .unwrap_or(false);
    let (origin, spec) = if npm_origin {
        (
            "npm",
            raw.package.clone().unwrap_or_else(|| raw.name.clone()),
        )
    } else if let Some(repo) = &raw.repo {
        let tag = raw.version.as_deref().filter(|v| {
            let head = v
                .strip_prefix('v')
                .unwrap_or(v)
                .split_once('-')
                .map(|(h, _)| h)
                .unwrap_or(v);
            let parts: Vec<&str> = head.split('.').collect();
            parts.len() >= 2 && parts[..2].iter().all(|s| s.parse::<u64>().is_ok())
        });
        let base = format!("https://github.com/{repo}.git");
        (
            "git",
            match tag {
                Some(t) => format!("{base}#{t}"),
                None => base,
            },
        )
    } else {
        ("git", raw.repo.clone().unwrap_or_default())
    };
    let detail_url = raw
        .repo
        .as_ref()
        .map(|r| format!("https://github.com/{r}"))
        .unwrap_or_default();
    CatalogItem {
        id: raw.id,
        name: raw.name,
        kind: raw.kind,
        description: raw.description.unwrap_or_default(),
        stars: raw.stars,
        forks: 0,
        downloads: raw.downloads,
        verified: raw.verified,
        repo: raw.repo,
        spec,
        origin: origin.to_string(),
        category: raw.category,
        version: raw.version.unwrap_or_default(),
        tags: Vec::new(),
        updated: String::new(),
        detail_url,
    }
}

/// GET a URL and return the body as text.
fn fetch_text(url: &str) -> Result<String, String> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e: ureq::Error| e.to_string())?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e: ureq::Error| e.to_string())
}

/// Fetch the community catalog, caching the normalized items for
/// CATALOG_TTL_SECS (`force` bypasses the cache). The dsh-plugin.org hub is
/// the primary source; the reference market listing is the fallback when the
/// hub is unreachable.
fn fetch_catalog(data_dir: &Path, force: bool) -> Result<Vec<CatalogItem>, String> {
    let cache = data_dir.join(CATALOG_CACHE_FILE);
    let fresh = !force
        && fs::metadata(&cache)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok().map(|e| e.as_secs() < CATALOG_TTL_SECS))
            .unwrap_or(false);
    if fresh {
        if let Ok(text) = fs::read_to_string(&cache) {
            if let Ok(items) = serde_json::from_str::<Vec<CatalogItem>>(&text) {
                return Ok(items);
            }
        }
    }
    let hub = fetch_text(HUB_CATALOG_URL).and_then(|body| {
        serde_json::from_str::<Vec<HubRaw>>(&body)
            .map_err(|e: serde_json::Error| e.to_string())
            .map(|raws| raws.into_iter().map(HubRaw::into_item).collect::<Vec<_>>())
    });
    let items = match hub {
        Ok(items) if !items.is_empty() => items,
        _ => {
            let body = fetch_text(MARKET_CATALOG_URL)?;
            let doc: CatalogDoc =
                serde_json::from_str(&body).map_err(|e: serde_json::Error| e.to_string())?;
            doc.items.into_iter().map(from_market_raw).collect()
        }
    };
    if fs::create_dir_all(data_dir).is_ok() {
        if let Ok(text) = serde_json::to_string(&items) {
            let _ = fs::write(&cache, text);
        }
    }
    Ok(items)
}

/// The full community catalog sorted by stars (`force` bypasses the cache).
/// Search and category filtering happen in the UI so filtering over the
/// cached list is instant.
pub fn catalog(data_dir: &Path, force: bool) -> Result<Vec<CatalogItem>, AppError> {
    let mut items = fetch_catalog(data_dir, force)
        .map_err(|e| AppError::Plugin(format!("目录获取失败：{e}")))?;
    items.sort_by(|a, b| b.stars.cmp(&a.stars));
    Ok(items)
}

/// Apply the store's plugin dependencies and bundle layers onto a profile
/// manifest. Returns whether anything changed. Pure (no fs, no pnpm), so
/// wiring is unit-testable without a toolchain.
fn wire_manifest(
    root: &mut serde_json::Value,
    specs: &BTreeMap<String, (String, bool)>,
    profile: &str,
) -> Result<bool, AppError> {
    let mut changed = false;
    let deps = root
        .get_mut("dependencies")
        .and_then(|d| d.as_object_mut())
        .ok_or_else(|| AppError::Plugin("profile manifest 缺少 dependencies".into()))?;
    for (name, (spec, _)) in specs {
        if deps.get(name).and_then(|s| s.as_str()) != Some(spec.as_str()) {
            deps.insert(name.clone(), serde_json::Value::String(spec.clone()));
            changed = true;
        }
    }
    deps.retain(|name, spec| {
        if !is_managed_spec(spec.as_str().unwrap_or("")) {
            return true; // 用户/CLI 管理的不动
        }
        if !specs.contains_key(name) {
            changed = true;
            return false;
        }
        true
    });

    // bundles：模板层与托管层重建，用户其他条目（CLI 添加等）原样保留。
    // 已卸载的托管插件：依赖被清退后其层必须同步清退，否则内核启动会因无法
    // 解析该 bundle 而失败——因此只保留依赖仍存在且非托管 spec 的层。
    let kept_user_bundles: std::collections::HashSet<String> = deps
        .iter()
        .filter(|(_, spec)| !is_managed_spec(spec.as_str().unwrap_or("")))
        .map(|(name, _)| name.clone())
        .collect();
    let managed_bundles: Vec<String> = specs
        .iter()
        .filter(|(_, (_, is_bundle))| *is_bundle)
        .map(|(name, _)| name.clone())
        .collect();
    let template: Vec<String> = template_bundles(profile);
    let mut next: Vec<String> = template.clone();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let bundles = root
        .get_mut("dsh")
        .and_then(|d| d.get_mut("profile"))
        .and_then(|p| p.get_mut("bundles"))
        .and_then(|b| b.as_array_mut())
        .ok_or_else(|| AppError::Plugin("profile manifest 缺少 dsh.profile.bundles".into()))?;
    for name in bundles.iter().filter_map(|b| b.as_str().map(String::from)) {
        if seen.contains(&name) || template.contains(&name) || managed_bundles.contains(&name) {
            continue;
        }
        if !kept_user_bundles.contains(&name) {
            changed = true;
            continue;
        }
        seen.insert(name.clone());
        next.push(name);
    }
    for name in &managed_bundles {
        if !next.contains(name) {
            next.push(name.clone());
        }
    }
    let current: Vec<String> = bundles
        .iter()
        .filter_map(|b| b.as_str().map(String::from))
        .collect();
    if next != current {
        *bundles = next.into_iter().map(serde_json::Value::String).collect();
        changed = true;
    }
    Ok(changed)
}

// --- orchestration ----------------------------------------------------------

/// Install a plugin: fetch into the store, install store deps in link mode,
/// materialize into every kernel, wire the active profile.
pub fn install(
    data_dir: &Path,
    settings: &settings::Settings,
    pnpm_exe: &Path,
    spec_str: &str,
    mode: &str,
    on_progress: &mut dyn FnMut(&str),
) -> Result<StoreItem, AppError> {
    let spec = parse_spec(spec_str)?;
    if store_item(data_dir, &spec.id).is_some() {
        return Err(AppError::Plugin(format!(
            "{} 已安装，请使用「更新」",
            spec.name
        )));
    }
    let mut item = fetch_into_store(data_dir, pnpm_exe, &spec, on_progress)?;
    item.mode = if mode == "copy" { "copy" } else { "link" }.to_string();
    if item.mode == "link" {
        // Ensure the store-level .npmrc exists before installing deps, so the
        // minimumReleaseAge exclusion is in place even if the store was created
        // before this fix was deployed.
        ensure_store_npmrc(data_dir).ok();
        install_store_deps(data_dir, pnpm_exe, &item.id, on_progress)?;
    }
    upsert_item(data_dir, item.clone())?;
    sync_kernels(data_dir, &item)?;
    on_progress("正在接线到 profile");
    ensure_wiring(data_dir, settings, pnpm_exe, on_progress)?;
    Ok(item)
}

/// Update one plugin: re-fetch the same source, refresh store deps, re-sync
/// all kernels, re-wire.
pub fn update(
    data_dir: &Path,
    settings: &settings::Settings,
    pnpm_exe: &Path,
    id: &str,
    on_progress: &mut dyn FnMut(&str),
) -> Result<StoreItem, AppError> {
    let item =
        store_item(data_dir, id).ok_or_else(|| AppError::Plugin("插件不在中央库中".into()))?;
    if item.pinned {
        return Err(AppError::Plugin(format!(
            "{} 已锁定版本 {}，如需升级请重新安装（不带版本号）",
            item.name, item.installed_version
        )));
    }
    let spec = parse_spec(&item.source)?;
    on_progress(&format!("正在更新 {}", item.name));
    let fetched = fetch_into_store(data_dir, pnpm_exe, &spec, on_progress)?;
    let mut updated = fetched;
    updated.mode = item.mode.clone();
    if updated.mode == "link" {
        ensure_store_npmrc(data_dir).ok();
        install_store_deps(data_dir, pnpm_exe, &updated.id, on_progress)?;
    }
    // Sync latest_version to what we just installed so the UI badge
    // clears immediately after a successful update. Without this, the
    // previous `check_updates` result lingers and the badge keeps
    // reporting the same phantom "newer version" the user just
    // installed. A later `check_updates` can still raise `latest_version`
    // when the remote has moved on since this fetch.
    updated.latest_version = Some(updated.installed_version.clone());
    upsert_item(data_dir, updated.clone())?;
    sync_kernels(data_dir, &updated)?;
    on_progress("正在同步 profile");
    ensure_wiring(data_dir, settings, pnpm_exe, on_progress)?;
    Ok(updated)
}

/// Remove a plugin everywhere: store, kernel materializations, profile wiring.
pub fn uninstall(
    data_dir: &Path,
    settings: &settings::Settings,
    pnpm_exe: &Path,
    id: &str,
    on_progress: &mut dyn FnMut(&str),
) -> Result<(), AppError> {
    store_item(data_dir, id).ok_or_else(|| AppError::Plugin("插件不在中央库中".into()))?;
    for version in kernel::list_installed(data_dir) {
        remove_materialized(data_dir, &version.version, id);
    }
    let _ = fs::remove_dir_all(store_plugin_dir(data_dir, id));
    remove_item(data_dir, id)?;
    ensure_wiring(data_dir, settings, pnpm_exe, on_progress)?;
    Ok(())
}

/// Re-apply the desired mode to every kernel and re-wire.
pub fn set_mode(
    data_dir: &Path,
    settings: &settings::Settings,
    pnpm_exe: &Path,
    id: &str,
    mode: &str,
    on_progress: &mut dyn FnMut(&str),
) -> Result<(), AppError> {
    if mode != "link" && mode != "copy" {
        return Err(AppError::Plugin("模式只能是 link 或 copy".into()));
    }
    let mut item =
        store_item(data_dir, id).ok_or_else(|| AppError::Plugin("插件不在中央库中".into()))?;
    item.mode = mode.to_string();
    upsert_item(data_dir, item.clone())?;
    if mode == "link" {
        install_store_deps(data_dir, pnpm_exe, id, on_progress)?;
    }
    sync_kernels(data_dir, &item)?;
    ensure_wiring(data_dir, settings, pnpm_exe, on_progress)?;
    Ok(())
}

/// Materialize everything and re-wire (the「同步」button).
pub fn sync_all(
    data_dir: &Path,
    settings: &settings::Settings,
    pnpm_exe: &Path,
    on_progress: &mut dyn FnMut(&str),
) -> Result<(), AppError> {
    let store = load_store(data_dir);
    for item in &store.items {
        sync_kernels(data_dir, item)?;
    }
    ensure_wiring(data_dir, settings, pnpm_exe, on_progress)?;
    Ok(())
}

/// Compose the UI status snapshot (no network).
pub fn status(data_dir: &Path, settings: &settings::Settings) -> PluginStatus {
    let store = load_store(data_dir);
    let active = kernel::read_active(data_dir);
    let profile_manifest = read_profile_json(data_dir, &settings.profile)
        .ok()
        .flatten();

    let mut rows = Vec::new();
    let mut updates = 0;
    for item in &store.items {
        let (actual_mode, synced) = match &active {
            Some(version) => {
                let meta = read_meta(data_dir, version, &item.id);
                let present = kernel_plugin_dir(data_dir, version, &item.id).exists();
                let current = meta
                    .as_ref()
                    .map(|m| m.version == item.installed_version)
                    .unwrap_or(false);
                (meta.map(|m| m.mode), present && current)
            }
            None => (None, false),
        };
        let expected_spec = active.as_ref().map(|version| {
            let prefix = if actual_mode.as_deref() == Some("copy") {
                SPEC_FILE
            } else {
                SPEC_LINK
            };
            let rel = relative_path(
                &profile_dir(data_dir, &settings.profile),
                &kernel_plugin_dir(data_dir, version, &item.id),
            );
            format!("{prefix}{}", spec_path_string(&rel))
        });
        let wired = profile_manifest
            .as_ref()
            .and_then(|m| m.get("dependencies"))
            .and_then(|d| d.get(&item.name))
            .and_then(|s| s.as_str())
            .map(|spec| {
                expected_spec.as_ref().map(|e| spec == e).unwrap_or(false)
                    || (!is_managed_spec(spec) && spec.contains(WIRED_MARK))
            })
            .unwrap_or(false);
        if item
            .latest_version
            .as_deref()
            .map(|l| is_newer_than(l, &item.installed_version, &item.origin, item.pinned))
            .unwrap_or(false)
        {
            updates += 1;
        }
        rows.push(PluginRow {
            id: item.id.clone(),
            name: item.name.clone(),
            origin: item.origin.clone(),
            source: item.source.clone(),
            installed_version: item.installed_version.clone(),
            latest_version: item.latest_version.clone(),
            pinned: item.pinned,
            desired_mode: item.mode.clone(),
            actual_mode,
            synced,
            wired,
            repo_url: item.repo_url.clone(),
            description: item.description.clone(),
            installed_at: item.installed_at.clone(),
            updated_at: item.updated_at.clone(),
        });
    }
    PluginStatus {
        rows,
        profile: settings.profile.clone(),
        active_kernel: active,
        updates,
        last_checked_at: store.last_checked_at,
        warning: store.warning,
    }
}

/// Resolve a link-mode plugin's peerDependencies from the ACTIVE kernel's
/// node_modules into the store dir, so the plugin's import walk finds the
/// same cordis/dsh-* instances the kernel uses. Recorded in
/// .dsh-peers.json keyed by kernel version, so a kernel switch re-runs it.
fn refresh_store_peers(data_dir: &Path, item: &StoreItem, active: &str) -> Result<(), AppError> {
    if item.mode != "link" {
        return Ok(());
    }
    let plugin_root = store_plugin_dir(data_dir, &item.id);
    let meta_path = plugin_root.join(".dsh-peers.json");
    let meta: serde_json::Value = fs::read_to_string(&meta_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if meta.get("kernel").and_then(|k| k.as_str()) == Some(active)
        && meta.get("peers").and_then(|p| p.as_array()).is_some()
    {
        return Ok(()); // 已为该内核解析过
    }
    let Ok(manifest) = read_plugin_manifest(&plugin_root) else {
        return Ok(());
    };
    let Some(peers) = manifest.get("peerDependencies").and_then(|p| p.as_object()) else {
        return Ok(());
    };
    let kernel_mm = kernel::kernel_dir(data_dir, active).join("node_modules");
    let mut linked: Vec<String> = Vec::new();
    for name in peers.keys() {
        let target = kernel_mm.join(name);
        if !target.exists() {
            continue;
        }
        let dest = plugin_root.join("node_modules").join(name);
        if dest.exists() {
            continue; // 已在库内安装（已发布或被 hoisted）
        }
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if make_dir_link(&target, &dest).is_err() {
            // 链接不可用（Windows 权限等）→ 复制一份，解析不依赖链接能力
            let _ = copy_tree(&target, &dest);
        }
        linked.push(name.clone());
    }
    let text = serde_json::json!({ "kernel": active, "peers": linked });
    if let Ok(text) = serde_json::to_string(&text) {
        let _ = fs::write(meta_path, text);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Unique throwaway home per test, removed on drop.
    struct TestHome(PathBuf);

    impl TestHome {
        fn new() -> Self {
            let nano = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let base =
                std::env::temp_dir().join(format!("dsh-plugins-test-{}", std::process::id()));
            let home = base.join(nano.to_string());
            fs::create_dir_all(&home).expect("test home");
            TestHome(home)
        }

        fn data_dir(&self) -> PathBuf {
            self.0.join("desktop")
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_npm_specs() {
        assert_eq!(parse_spec("dsh-market").unwrap().origin, "npm");
        let scoped = parse_spec("@ace-zone/dsh-market").unwrap();
        assert_eq!(scoped.origin, "npm");
        assert_eq!(scoped.source, "@ace-zone/dsh-market");
        assert_eq!(scoped.pin, None);
        let pinned = parse_spec("@ace-zone/dsh-market@0.1.66").unwrap();
        assert_eq!(pinned.pin.as_deref(), Some("0.1.66"));
        assert_eq!(pinned.id, "@ace-zone__dsh-market");
        let unpinned = parse_spec("dsh-market@1.2.3").unwrap();
        assert_eq!(unpinned.source, "dsh-market");
        assert_eq!(unpinned.pin.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn validate_plugin_checks_the_load_contract() {
        let home = TestHome::new();
        let dir = home.0.join("plugin");
        fs::create_dir_all(&dir).expect("plugin dir");

        // 没有 package.json：拒绝
        assert!(validate_plugin(&dir).is_err());

        // bundle 层声明了 patch 但文件缺失：拒绝
        fs::write(
            dir.join("package.json"),
            r#"{"name":"p","dsh":{"bundle":{"patch":"./cordis.patch.yml"}}}"#,
        )
        .expect("manifest");
        assert!(validate_plugin(&dir).is_err());

        // patch 文件补齐但无运行时入口：仍拒绝，bundle 层不能替代 main/exports
        fs::write(dir.join("cordis.patch.yml"), "patches: []\n").expect("patch");
        assert!(validate_plugin(&dir).is_err());

        // 普通依赖型插件：main 指向真实文件才放行
        fs::write(
            dir.join("package.json"),
            r#"{"name":"p","main":"lib/index.js"}"#,
        )
        .expect("manifest");
        assert!(validate_plugin(&dir).is_err());
        fs::create_dir_all(dir.join("lib")).expect("lib");
        fs::write(dir.join("lib/index.js"), "module.exports = {}\n").expect("entry");
        assert!(validate_plugin(&dir).is_ok());

        // bundle 层 + 有效运行时入口：放行
        fs::write(
            dir.join("package.json"),
            r#"{"name":"p","main":"lib/index.js","dsh":{"bundle":{"patch":"./cordis.patch.yml"}}}"#,
        )
        .expect("manifest");
        assert!(validate_plugin(&dir).is_ok());

        // exports 入口存在即放行（Node 自己解析其目标）
        fs::write(
            dir.join("package.json"),
            r#"{"name":"p","exports":"./lib/index.js"}"#,
        )
        .expect("manifest");
        assert!(validate_plugin(&dir).is_ok());

        // 既无 bundle 也无入口：拒绝
        fs::write(dir.join("package.json"), r#"{"name":"p"}"#).expect("manifest");
        assert!(validate_plugin(&dir).is_err());
    }

    #[test]
    fn hub_entry_normalizes_to_catalog_item() {
        let raw: HubRaw = serde_json::from_str(
            r#"{"s":"modlens","o":"liustack","n":"modlens","vr":"v3.22.1","c":"tools",
                "t":["vision"],"d":"desc","r":"liustack/modlens","v":"verified",
                "u":"2026-08-20T20:05:55Z","sg":3497,"fk":95}"#,
        )
        .expect("hub raw");
        let item = raw.into_item();
        assert_eq!(item.origin, "git");
        assert_eq!(item.spec, "https://github.com/liustack/modlens.git");
        assert_eq!(item.version, "v3.22.1");
        assert_eq!(item.stars, 3497);
        assert_eq!(item.forks, 95);
        assert!(item.verified);
        assert_eq!(item.category, "tools");
        assert_eq!(
            item.detail_url,
            "https://dsh-plugin.org/zh/plugins/liustack/modlens"
        );

        // 无 repo 的条目回退 npm 安装
        let raw: HubRaw = serde_json::from_str(r#"{"n":"pkg","d":"x"}"#).expect("hub raw");
        let item = raw.into_item();
        assert_eq!(item.origin, "npm");
        assert_eq!(item.spec, "pkg");
        assert!(!item.verified);
    }

    #[test]
    fn parses_git_specs() {
        let url = parse_spec("https://github.com/losebird/dsh-plugin-market").unwrap();
        assert_eq!(url.origin, "git");
        assert!(url.source.starts_with("https://"));
        let pinned = parse_spec("https://github.com/o/r.git#v1.2.3").unwrap();
        assert_eq!(pinned.pin.as_deref(), Some("v1.2.3"));
        let ssh = parse_spec("git@github.com:o/r.git").unwrap();
        assert_eq!(ssh.origin, "git");
        let shorthand = parse_spec("losebird/dsh-plugin-market").unwrap();
        assert_eq!(shorthand.origin, "git");
        assert_eq!(
            shorthand.source,
            "https://github.com/losebird/dsh-plugin-market.git"
        );
        assert_eq!(shorthand.id, "losebird__dsh-plugin-market");
    }

    #[test]
    fn rejects_path_traversal_ids() {
        assert!(id_for_name("a/../b").is_err());
        assert!(id_for_name("..").is_err());
        assert!(id_for_name("").is_err());
        assert!(id_for_name("a//b").is_err());
    }

    #[test]
    fn picks_latest_tag() {
        let tags = ["v1.2.3", "v1.2.0", "v0.9.0", "v2.0.0-rc.1"];
        assert_eq!(
            latest_tag(tags.iter().copied()).as_deref(),
            Some("v2.0.0-rc.1")
        );
        assert_eq!(
            latest_tag(["1.2.3", "1.10.0"].iter().copied()).as_deref(),
            Some("1.10.0")
        );
        assert_eq!(latest_tag(["not-a-version"].iter().copied()), None);
    }

    #[test]
    fn detects_semver_shape() {
        // Two numeric segments is the bar the tag filter and update
        // comparator both rely on; everything else is treated as a hash.
        assert!(looks_like_semver("v0.15.0"));
        assert!(looks_like_semver("0.15.0"));
        assert!(looks_like_semver("1.2.3-rc.1"));
        assert!(!looks_like_semver("v646c91c"));
        assert!(!looks_like_semver("head"));
        assert!(!looks_like_semver("1"));
        assert!(!looks_like_semver(""));
    }

    #[test]
    fn newer_than_handles_hash_vs_semver() {
        // npm / pinned git keep the semver ranking. The unpinned git
        // branch now also stores the highest remote tag (via
        // `fetch_git`), so it joins the same semver ranking path.
        assert!(is_newer_than("v0.15.0", "v0.14.0", "npm", false));
        assert!(!is_newer_than("v0.14.0", "v0.15.0", "npm", false));
        assert!(is_newer_than("v1.0.0", "v0.15.0", "git", true));
        assert!(is_newer_than("v0.16.0", "v0.15.0", "git", false));
        assert!(!is_newer_than("v0.15.0", "v0.15.0", "git", false));

        // Fallback path: unpinned git-origin whose repo has no usable
        // semver tags records `installed_version` as the HEAD short
        // hash. The remote `latest` is a tag, so a plain semver compare
        // would say Greater purely on segment count. Use string equality
        // instead so a fresh tag does not look "newer" forever after.
        assert!(!is_newer_than("v0.15.0", "v646c91c", "git", false));
        assert!(is_newer_than("vNEW1", "v646c91c", "git", false));
        assert!(!is_newer_than("v646c91c", "v646c91c", "git", false));
        // Pinned with a hash-shaped latest never reaches the special
        // branch; cmp_versions ranks the hash below any semver tag.
        assert!(is_newer_than("v0.15.0", "v646c91c", "git", true));
    }

    #[test]
    fn computes_relative_paths() {
        let from = Path::new("/home/u/.dsh/profiles/web");
        let to = Path::new("/home/u/.dsh/desktop/kernels/0.1.1/plugins/x");
        assert_eq!(
            relative_path(from, to).to_string_lossy(),
            "../../desktop/kernels/0.1.1/plugins/x"
        );
        assert_eq!(relative_path(from, from).to_string_lossy(), "");
    }

    #[test]
    fn store_round_trips() {
        let home = TestHome::new();
        let data_dir = home.data_dir();
        let item = StoreItem {
            id: "test-plugin-1".into(),
            name: "test-plugin".into(),
            origin: "npm".into(),
            source: "test-plugin".into(),
            installed_version: "1.0.0".into(),
            latest_version: None,
            mode: "link".into(),
            pinned: false,
            installed_at: "1".into(),
            updated_at: "2".into(),
            repo_url: None,
            description: None,
        };
        upsert_item(&data_dir, item.clone()).expect("save");
        let loaded = load_store(&data_dir);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].id, "test-plugin-1");
        assert!(store_dir(&data_dir).starts_with(home.0.as_path()));
    }

    #[test]
    fn materialize_link_then_copy() {
        let home = TestHome::new();
        let data_dir = home.data_dir();
        let id = "mat-plugin";
        let source = store_plugin_dir(&data_dir, id);
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("package.json"), "{}").unwrap();
        let item = StoreItem {
            id: id.into(),
            name: "mat-plugin".into(),
            origin: "npm".into(),
            source: "mat-plugin".into(),
            installed_version: "1.0.0".into(),
            latest_version: None,
            mode: "link".into(),
            pinned: false,
            installed_at: String::new(),
            updated_at: String::new(),
            repo_url: None,
            description: None,
        };
        let version = "0.1.1";
        let actual = materialize_one(&data_dir, version, &item).expect("materialize");
        // 链接失败（Windows 无开发者模式、受限文件系统、沙箱）会降级为 copy，
        // 两种结果都是合法行为；能链接时必须真的是链接。
        assert!(actual == "link" || actual == "copy");
        let target = kernel_plugin_dir(&data_dir, version, id);
        assert!(target.exists());
        if actual == "link" {
            assert!(target.is_symlink());
        }
        let actual = materialize_one(&data_dir, version, &item).expect("idempotent");
        assert!(actual == "link" || actual == "copy");

        // copy 模式覆盖
        let mut copy_item = item.clone();
        copy_item.mode = "copy".to_string();
        let actual = materialize_one(&data_dir, version, &copy_item).expect("copy");
        assert_eq!(actual, "copy");
        assert!(target.join("package.json").is_file());
        let meta = read_meta(&data_dir, version, id).expect("meta");
        assert_eq!(meta.mode, "copy");
        assert_eq!(meta.version, "1.0.0");
    }

    #[test]
    fn refreshes_peers_from_active_kernel() {
        let home = TestHome::new();
        let data_dir = home.data_dir();
        let id = "peer-plugin";
        let version = "2.0.0";
        // 假内核：node_modules 里有插件声明但中央库没有的 peer
        let kernel_mm = kernel::kernel_dir(&data_dir, version).join("node_modules");
        fs::create_dir_all(kernel_mm.join("@deepseek-ai/dsh-base")).unwrap();
        fs::write(kernel_mm.join("@deepseek-ai/dsh-base/package.json"), "{}").unwrap();
        fs::write(data_dir.join("active.txt"), format!("{version}\n")).unwrap();
        let plugin_root = store_plugin_dir(&data_dir, id);
        fs::create_dir_all(&plugin_root).unwrap();
        let manifest = serde_json::json!({
            "name": "peer-plugin",
            "peerDependencies": { "@deepseek-ai/dsh-base": "*" },
        });
        fs::write(
            plugin_root.join("package.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let item = StoreItem {
            id: id.into(),
            name: "peer-plugin".into(),
            origin: "npm".into(),
            source: "peer-plugin".into(),
            installed_version: "1.0.0".into(),
            latest_version: None,
            mode: "link".into(),
            pinned: false,
            installed_at: String::new(),
            updated_at: String::new(),
            repo_url: None,
            description: None,
        };
        refresh_store_peers(&data_dir, &item, version).expect("peers");
        let dest = plugin_root.join("node_modules/@deepseek-ai/dsh-base");
        assert!(dest.exists(), "peer 应被链接/复制进中央库");
        assert!(dest.join("package.json").is_file());
        // 幂等：同内核再跑一次不重复（存在即跳过）
        refresh_store_peers(&data_dir, &item, version).expect("peers again");
        assert!(dest.exists());
    }

    #[test]
    fn wire_manifest_applies_and_prunes() {
        let mut root = serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {
                "other": "1.0.0",
                "old-plugin": "link:../../desktop/kernels/1.0.0/plugins/old-plugin",
            },
            "dsh": {
                "profile": {
                    "bundles": ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app", "old-plugin"],
                },
            },
        });
        let mut specs = BTreeMap::new();
        specs.insert(
            "new-plugin".to_string(),
            (
                "link:../../desktop/kernels/9.9.9/plugins/new-plugin".to_string(),
                true,
            ),
        );
        specs.insert(
            "plain-plugin".to_string(),
            (
                "link:../../desktop/kernels/9.9.9/plugins/plain-plugin".to_string(),
                false,
            ),
        );

        let changed = wire_manifest(&mut root, &specs, "web").expect("wire");
        assert!(changed);
        let deps = root["dependencies"].as_object().expect("deps");
        assert_eq!(
            deps["new-plugin"].as_str().unwrap(),
            "link:../../desktop/kernels/9.9.9/plugins/new-plugin"
        );
        assert!(deps.contains_key("plain-plugin"));
        assert!(deps.contains_key("other")); // 用户/CLI 条目保留
        assert!(!deps.contains_key("old-plugin")); // 已卸载条目清退
        let bundles: Vec<&str> = root["dsh"]["profile"]["bundles"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b.as_str())
            .collect();
        assert!(bundles.contains(&"@deepseek-ai/dsh-base"));
        assert!(bundles.contains(&"new-plugin"));
        assert!(!bundles.contains(&"plain-plugin")); // 非 bundle 不进层
        assert!(!bundles.contains(&"old-plugin"));

        let changed = wire_manifest(&mut root, &specs, "web").expect("wire again");
        assert!(!changed);
    }

    #[test]
    fn spec_path_uses_forward_slashes() {
        #[cfg(windows)]
        let rel = PathBuf::from("..\\..\\desktop\\kernels\\1\\plugins\\x");
        #[cfg(not(windows))]
        let rel = PathBuf::from("../../desktop/kernels/1/plugins/x");
        assert_eq!(spec_path_string(&rel), "../../desktop/kernels/1/plugins/x");
    }

    #[test]
    fn settings_profile_defaults_to_web() {
        let s = settings::Settings::default();
        assert_eq!(s.profile, DEFAULT_PROFILE);
        assert_eq!(DEFAULT_PROFILE, "web");
    }

    #[test]
    fn status_flags_stale_materialization() {
        let home = TestHome::new();
        let data_dir = home.data_dir();
        let version = "1.0.0";
        fs::create_dir_all(kernel::kernel_dir(&data_dir, version)).unwrap();
        fs::write(data_dir.join("active.txt"), format!("{version}\n")).unwrap();
        upsert_item(
            &data_dir,
            StoreItem {
                id: "stale-plugin".into(),
                name: "stale-plugin".into(),
                origin: "npm".into(),
                source: "stale-plugin".into(),
                installed_version: "2.0.0".into(),
                latest_version: Some("3.0.0".into()),
                mode: "link".into(),
                pinned: false,
                installed_at: String::new(),
                updated_at: String::new(),
                repo_url: None,
                description: None,
            },
        )
        .expect("save");
        let settings = settings::Settings::default();
        let view = status(&data_dir, &settings);
        assert_eq!(view.rows.len(), 1);
        assert!(!view.rows[0].synced);
        assert_eq!(view.updates, 1);
        assert_eq!(view.rows[0].latest_version.as_deref(), Some("3.0.0"));
    }
}
