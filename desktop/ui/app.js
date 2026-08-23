'use strict';

// dsh-desktop management UI.
// Talks to the shell only through Tauri commands (window.__TAURI__.core),
// so the whole page is static and needs no bundler.

const core = window.__TAURI__ && window.__TAURI__.core;

function invoke(cmd, args) {
  if (!core) {
    return Promise.reject(new Error('Tauri bridge 未注入（请通过桌面应用运行本页面）'));
  }
  return core.invoke(cmd, args || {});
}

const $ = (id) => document.getElementById(id);

// Same-value textContent writes still dirty layout, and the status poll
// re-renders every 2.5s; skip the write when nothing changed.
function setText(id, s) {
  const node = $(id);
  if (node.textContent !== s) {
    node.textContent = s;
  }
}

// createElement + className + textContent in one call; mkBtn adds the
// type and click handler every button here sets anyway.
function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) {
    node.className = cls;
  }
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

function mkBtn(label, onClick, cls) {
  const btn = el('button', cls, label);
  btn.type = 'button';
  btn.addEventListener('click', onClick);
  return btn;
}

// --- toast ---------------------------------------------------------------

let toastTimer = null;
function toast(msg, ms) {
  const toastEl = $('toast');
  toastEl.textContent = msg;
  toastEl.classList.remove('hidden');
  if (toastTimer) {
    clearTimeout(toastTimer);
  }
  toastTimer = setTimeout(() => toastEl.classList.add('hidden'), ms || 3200);
}

// --- progress --------------------------------------------------------------

function setProgress(text) {
  $('progressText').textContent = text;
  $('progress').classList.remove('hidden');
}

function hideProgress() {
  $('progress').classList.add('hidden');
}

// --- install log stream ------------------------------------------------------
//
// The Rust side forwards every pnpm output line over the command channel.
// Lines arrive as raw terminal text, so ANSI escape sequences are stripped
// for display; the full verbatim log stays in <app_data>/logs/install-*.log.

const INSTALL_LOG_MAX_LINES = 400;

let installLogLines = [];
let installLogScheduled = false;
let installFailed = false;

function stripAnsi(text) {
  // ESC [ ... letter (CSI), ESC ] ... BEL/ESC (OSC), and lone ESC followers.
  return text
    .replace(/\x1B\[[0-?]*[ -/]*[@-~]/g, '')
    .replace(/\x1B\][^\x07\x1B]*(?:\x07|\x1B\\)/g, '')
    .replace(/\x1B[@-_]/g, '');
}

function renderInstallLog() {
  const box = $('installLog');
  const pre = $('installLogText');
  pre.textContent = installLogLines.join('\n');
  box.classList.remove('hidden');
  box.scrollTop = box.scrollHeight;
}

function appendInstallLog(line) {
  if (!line) {
    return;
  }
  installLogLines.push(stripAnsi(line));
  if (installLogLines.length > INSTALL_LOG_MAX_LINES) {
    installLogLines.splice(0, installLogLines.length - INSTALL_LOG_MAX_LINES);
  }
  if (!installLogScheduled) {
    installLogScheduled = true;
    requestAnimationFrame(() => {
      installLogScheduled = false;
      renderInstallLog();
    });
  }
}

function resetInstallLog() {
  installLogLines = [];
  $('installLogText').textContent = '';
  $('installLog').classList.add('hidden');
  $('progressActions').classList.add('hidden');
}

// --- busy guard ------------------------------------------------------------

const busyButtons = new Set();
function setBusy(on) {
  if (on) {
    document.querySelectorAll('button').forEach((btn) => {
      if (!btn.dataset.wasDisabled) {
        btn.dataset.wasDisabled = String(btn.disabled);
      }
      btn.disabled = true;
      busyButtons.add(btn);
    });
    return;
  }
  // Restore from the Set itself, not a fresh DOM query: panels re-render via
  // innerHTML while busy, and the detached originals would stay in the Set
  // forever, leaving busyButtons non-empty and every guarded button (including
  // the workbench toggle) stuck disabled.
  busyButtons.forEach((btn) => {
    btn.disabled = btn.dataset.wasDisabled === 'true';
    delete btn.dataset.wasDisabled;
  });
  busyButtons.clear();
  // The workbench buttons' disabled state is owned by syncWorkbenchButtons;
  // recompute so a stale pre-busy snapshot cannot win over the real status.
  syncWorkbenchButtons();
}

// --- rendering -------------------------------------------------------------

let releases = [];
let currentView = null;
// Last observed kernel running state. refreshAll, waitForRunning, and the
// polling loop all keep it in sync so only an externally-caused transition
// to running raises the「内核已就绪」toast (the start orchestration toasts
// itself); initializing to false alone would misfire when the kernel was
// already running before this page loaded.
let lastRunning = false;

function renderStatus(view) {
  currentView = view;
  const { kernel, node, settings } = view;

  const pill = $('statusPill');
  const dot = $('statusDot');
  if (kernel.running) {
    pill.classList.remove('hidden');
    dot.className = 'dot ok';
    setText('statusText', '运行中');
  } else if (kernel.active && kernel.active_installed) {
    dot.className = 'dot bad';
    setText('statusText', '已停止');
  } else {
    dot.className = '';
    setText('statusText', '未安装');
  }

  setText('kernelRunning', kernel.running ? '运行中' : '未运行');
  setText('kernelActive', kernel.active || '（未选择）');
  setText('kernelUrl', kernel.running ? 'http://127.0.0.1:' + kernel.port : '—');
  setText('kernelNode', node.ok
    ? [node.path, node.version].filter(Boolean).join('  ')
    : '未检测到可用 Node（' + node.reason + '）');
  setText('kernelHome', kernel.data_dir);
  setText('shellVersion', 'v' + view.shell_version);

  setText('updateInstalled', String((kernel.installed || []).length) + ' 个');

  // The status poll re-renders every 2.5s; never clobber a field the user
  // is editing right now, or an in-flight edit would silently revert.
  if (document.activeElement !== $('setPort')) {
    $('setPort').value = String(settings.port);
  }
  if (document.activeElement !== $('setProfile')) {
    $('setProfile').value = settings.profile || '';
  }

  setText('nodeHint', node.ok
    ? 'node ' + node.version + ' 满足 dsh 要求（^22.19 || >=24）'
    : node.reason);

  syncWorkbenchButtons();
}

// --- workbench toggle state machine -----------------------------------------
//
// The kernel is an implementation detail of the workbench: one toggle button
// orchestrates start (kernel + wait-ready + open window) and stop (window +
// kernel). `starting` marks the local orchestration window between the click
// and the port answering; the 2.5s status poll keeps calling this so buttons
// track real state without clobbering the in-flight "正在启动…" phase.

let starting = false;

function syncWorkbenchButtons() {
  const k = currentView && currentView.kernel;
  const toggle = $('btnToggle');
  const openWindow = $('btnOpenWindow');
  const hint = $('startHint');
  const busy = busyButtons.size > 0;
  const running = Boolean(k && k.running);
  const canStart = Boolean(k && k.active && k.active_installed);

  // The toggle carries an SVG icon, so the label span owns the text.
  if (starting) {
    toggle.disabled = true;
    setText('btnToggleLabel', '正在启动…');
  } else if (running) {
    toggle.disabled = busy;
    setText('btnToggleLabel', '关闭工作台');
  } else {
    toggle.disabled = !canStart || busy;
    setText('btnToggleLabel', '启动工作台');
  }
  openWindow.disabled = !running || starting || busy;
  hint.classList.toggle('hidden', starting || running || canStart);
}

// installedSet is hoisted by the caller: one Set per render pass instead of
// one per release row.
function badgeFor(version, installedSet) {
  const k = currentView && currentView.kernel;
  if (k && k.active === version) {
    return el('span', 'badge active', '当前使用');
  }
  if (installedSet.has(version)) {
    return el('span', 'badge installed', '已安装');
  }
  return null;
}

// Two-step confirmation for destructive actions: WKWebView does not support
// window.confirm, so removal uses an in-page armed state instead. Only one
// button is armed at a time; arming another resets the previous one.
let armed = null;

function disarmConfirm() {
  if (!armed) {
    return;
  }
  clearTimeout(armed.timer);
  armed.btn.textContent = armed.idleLabel;
  armed.btn.classList.remove('armed');
  armed = null;
}

function armConfirm(btn, opts) {
  if (armed && armed.btn === btn) {
    // 第二次点击即确认：保持 armed 文案（按钮随即禁用，操作完成后整表重渲染），
    // 只摘掉 armed 态并执行。
    clearTimeout(armed.timer);
    armed.btn.classList.remove('armed');
    armed = null;
    opts.onConfirm(btn);
    return;
  }
  disarmConfirm();
  armed = { btn, idleLabel: opts.idleLabel, timer: setTimeout(disarmConfirm, 3200) };
  btn.textContent = opts.armedLabel;
  btn.classList.add('armed');
}

function proceedRemove(version, btn) {
  btn.disabled = true;
  setBusy(true);
  invoke('remove_version', { version })
    .then(() => {
      toast('已删除版本 ' + version);
      return refreshAll();
    })
    .catch((e) => toast('删除失败：' + e, 5000))
    .finally(() => setBusy(false));
}

function installedVersions() {
  const k = currentView && currentView.kernel;
  const set = new Set();
  if (k) {
    k.installed.forEach((v) => set.add(v.version));
  }
  return set;
}

function renderReleases() {
  const list = $('releaseList');
  const warn = $('releaseWarning');
  warn.classList.add('hidden');
  if (releases.length === 0) {
    list.innerHTML = '<p class="muted">点击「检查更新」获取官方发布列表。</p>';
    return;
  }
  list.innerHTML = '';
  const installedSet = installedVersions();
  const k = currentView && currentView.kernel;
  releases.forEach((r) => {
    const row = el('div', 'release-row');
    const ver = el('span', 'release-ver', r.version);
    const actions = el('span', 'release-actions');
    const badge = badgeFor(r.version, installedSet);
    if (badge) {
      actions.appendChild(badge);
    }

    const installed = installedSet.has(r.version);
    const isActive = k && k.active === r.version;
    if (!installed) {
      actions.appendChild(mkBtn('安装', () => installVersion(r.version)));
    } else if (!isActive) {
      actions.appendChild(mkBtn('切换', () => activateVersion(r.version)));
    }

    if (r.prerelease) {
      actions.appendChild(el('span', 'badge', '预发布'));
    }

    row.appendChild(ver);
    row.appendChild(actions);
    list.appendChild(row);
  });
}

function renderInstalled() {
  const list = $('installedList');
  const k = currentView && currentView.kernel;
  if (!k || k.installed.length === 0) {
    list.innerHTML = '<p class="muted">尚未安装任何内核。</p>';
    return;
  }
  list.innerHTML = '';
  k.installed.forEach((v) => {
    const row = el('div', 'installed-row');
    const ver = el('span', 'release-ver', v.version);
    const actions = el('span', 'release-actions');
    if (v.active) {
      actions.appendChild(el('span', 'badge active', '当前使用'));
    }
    if (!v.active) {
      actions.appendChild(mkBtn('删除', (ev) => armConfirm(ev.currentTarget, {
        armedLabel: '确认删除？',
        idleLabel: '删除',
        onConfirm: (btn) => proceedRemove(v.version, btn),
      }), 'danger'));
    }

    row.appendChild(ver);
    row.appendChild(actions);
    list.appendChild(row);
  });
}

// --- actions ----------------------------------------------------------------

async function refreshAll() {
  return invoke('get_status')
    .then((view) => {
      lastRunning = view.kernel.running;
      renderStatus(view);
      renderReleases();
      renderInstalled();
    })
    .then(() => (window.__dshPluginsRefresh ? window.__dshPluginsRefresh() : null))
    .catch((e) => toast('读取状态失败：' + e, 5000));
}

function checkUpdates() {
  setBusy(true);
  invoke('fetch_releases')
    .then((list) => {
      releases = list.releases || [];
      const warn = $('releaseWarning');
      if (list.warning) {
        warn.textContent = list.warning;
        warn.classList.remove('hidden');
      } else {
        warn.classList.add('hidden');
      }
      renderReleases();
      if (releases.length === 0) {
        toast('没有获取到官方发布，请稍后再试', 4000);
      }
    })
    .catch((e) => {
      releases = [];
      renderReleases();
      toast('获取发布失败：' + e, 6000);
    })
    .finally(() => setBusy(false));
}

// --- shell self-update -------------------------------------------------------
// The shell updates itself from the latest published GitHub release (the
// kernel has its own version menu). A background check runs at startup and
// raises the banner via `shell-update-available`; the button re-checks on
// demand, and「更新并重启」downloads, verifies, installs, and relaunches.

function showShellUpdateBanner(version) {
  const banner = $('shellUpdateBanner');
  banner.textContent = '发现桌面端新版本 v' + version + '（当前 v' + (currentView ? currentView.shell_version : '?') + '）';
  banner.classList.remove('hidden');
  $('btnShellInstall').classList.remove('hidden');
}

// Open the shell's data directory in the OS file manager (Finder on
// macOS, Explorer on Windows). The Rust command reads AppState.data_dir
// and hands it to the opener plugin, which dispatches per-OS.
function openDataDir() {
  invoke('open_data_dir').catch((e) => toast('打开数据目录失败：' + e, 5000));
}

function checkShellUpdate(manual) {
  invoke('check_shell_update')
    .then((info) => {
      if (info.available) {
        showShellUpdateBanner(info.available);
      } else if (manual) {
        toast('桌面端已是最新（v' + info.current + '）');
      }
    })
    .catch((e) => {
      if (manual) {
        toast('检查桌面端更新失败：' + e, 5000);
      }
    });
}

function installShellUpdate() {
  const channel = new core.Channel();
  channel.onmessage = (msg) => {
    $('shellUpdateBanner').textContent = msg;
  };
  $('btnShellInstall').disabled = true;
  invoke('install_shell_update', { onEvent: channel })
    .catch((e) => {
      toast('桌面端更新失败：' + e, 6000);
      $('btnShellInstall').disabled = false;
    });
  // On success the app restarts into the new version; nothing else to do.
}

// installVersion shares the plugin progress plumbing (defined in plugins.js,
// which loads after this file — fine, it is only called from click handlers).
function installVersion(version) {
  return withPluginProgress(
    {
      cmd: 'install_kernel',
      start: '正在安装 ' + version + ' …',
      done: '版本 ' + version + ' 安装完成',
      fail: '安装失败',
      failToast: '安装失败，详情见进度窗口与日志'
    },
    (channel) => ({ version, onEvent: channel })
  );
}

function closeProgress() {
  installFailed = false;
  hideProgress();
  resetInstallLog();
}

function activateVersion(version) {
  setBusy(true);
  invoke('activate_version', { version })
    .then(() => {
      toast('已切换活动版本为 ' + version + '（下次启动生效）');
      return refreshAll();
    })
    .catch((e) => toast('切换失败：' + e, 5000))
    .finally(() => setBusy(false));
}

// Poll until the kernel port answers; start_kernel returns right after spawn,
// so readiness can only be observed through get_status.
const START_TIMEOUT_MS = 60000;

function waitForRunning(deadline) {
  return invoke('get_status').then((view) => {
    lastRunning = view.kernel.running;
    renderStatus(view);
    if (view.kernel.running) {
      return true;
    }
    if (Date.now() > deadline) {
      return false;
    }
    return new Promise((resolve) => setTimeout(resolve, 1000)).then(() => waitForRunning(deadline));
  });
}

function startWorkbench() {
  starting = true;
  syncWorkbenchButtons();
  invoke('start_kernel')
    .then(() => waitForRunning(Date.now() + START_TIMEOUT_MS))
    .then((ready) => {
      if (!ready) {
        throw new Error('等待内核就绪超时（' + START_TIMEOUT_MS / 1000 + ' 秒），详情见日志');
      }
      return invoke('open_harness');
    })
    .then(() => toast('工作台已启动'))
    .catch((e) => {
      toast('启动失败：' + e, 8000);
      // 失败路径的出口：直接展示日志，无需用户再找入口。
      showLogs();
    })
    .finally(() => {
      starting = false;
      refreshAll();
    });
}

// 自绘确认框（WKWebView 无原生 confirm）。同时只允许一个待决确认。
let confirmResolve = null;

function confirmDialog(title, text, okLabel) {
  $('confirmTitle').textContent = title;
  $('confirmText').textContent = text;
  $('btnConfirmOk').textContent = okLabel || '确认';
  $('confirmModal').classList.remove('hidden');
  return new Promise((resolve) => {
    confirmResolve = resolve;
  });
}

function settleConfirm(ok) {
  $('confirmModal').classList.add('hidden');
  if (confirmResolve) {
    const resolve = confirmResolve;
    confirmResolve = null;
    resolve(ok);
  }
}

function stopWorkbench() {
  const proceed = () => {
    setBusy(true);
    invoke('stop_kernel')
      .then(() => {
        toast('工作台已关闭');
        return refreshAll();
      })
      .catch((e) => toast('关闭失败：' + e, 5000))
      .finally(() => setBusy(false));
  };
  const running = currentView && currentView.kernel && currentView.kernel.running;
  if (!running) {
    // 内核未运行：只是关掉残留的工作台窗口，无需确认
    proceed();
    return;
  }
  // 运行中才确认：内核可能正在思考，停止会中断未完成的回复
  confirmDialog(
    '确认停止内核？',
    '内核正在运行。如果它正在思考或处理任务，停止将中断未完成的回复。',
    '停止内核'
  ).then((ok) => {
    if (ok) {
      proceed();
    }
  });
}

function openHarnessWindow() {
  invoke('open_harness').catch((e) => toast('无法打开工作台窗口：' + e, 5000));
}

// --- logs modal -----------------------------------------------------------
//
// The modal lists every *.log file under <data_dir>/logs/ as a tab. The
// first tab (newest file by name — `kernel.log` for live output) is read
// when the modal opens; switching tabs is on-demand and a refresh button
// re-reads the active tab. Tabs scroll horizontally when they don't fit
// on the 480px window so the panel stays single-row.

let activeLogName = null;

function formatLogSize(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / (1024 * 1024)).toFixed(2) + ' MB';
}

function renderLogTabs(files, selectedName) {
  const strip = $('logTabs');
  strip.innerHTML = '';
  if (!files || !files.length) {
    const empty = document.createElement('span');
    empty.className = 'log-tab-size';
    empty.textContent = '（暂无日志文件）';
    strip.appendChild(empty);
    return;
  }
  files.forEach((f) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'log-tab';
    btn.role = 'tab';
    btn.dataset.name = f.name;
    btn.setAttribute('aria-selected', f.name === selectedName ? 'true' : 'false');
    btn.title = f.name;
    const label = document.createElement('span');
    label.textContent = f.name;
    btn.appendChild(label);
    if (typeof f.size === 'number') {
      const sz = document.createElement('span');
      sz.className = 'log-tab-size';
      sz.textContent = formatLogSize(f.size);
      btn.appendChild(sz);
    }
    btn.addEventListener('click', () => switchLogTab(f.name));
    strip.appendChild(btn);
  });
}

function selectLogTab(name) {
  document.querySelectorAll('#logTabs .log-tab').forEach((el) => {
    el.setAttribute('aria-selected', el.dataset.name === name ? 'true' : 'false');
  });
}

function loadActiveLog() {
  if (!activeLogName) {
    $('logContent').textContent = '（暂无日志）';
    return;
  }
  const target = activeLogName;
  $('logContent').textContent = '读取中…';
  invoke('read_log_file', { name: target })
    .then((text) => {
      // Guard against a tab switch happening mid-fetch: only paint the
      // content if the user hasn't navigated away.
      if (activeLogName !== target) return;
      $('logContent').textContent = text || '（暂无内容）';
    })
    .catch((e) => {
      $('logContent').textContent = '读取失败：' + e;
    });
}

function switchLogTab(name) {
  if (name === activeLogName) {
    loadActiveLog();
    return;
  }
  activeLogName = name;
  selectLogTab(name);
  loadActiveLog();
}

function refreshLogTabs() {
  // Re-list files so new install logs and rotated kernel logs appear, then
  // keep the user on the same tab if it still exists; otherwise fall back
  // to the first tab (or the empty state).
  invoke('list_log_files')
    .then((files) => {
      const names = (files || []).map((f) => f.name);
      const keep = activeLogName && names.includes(activeLogName) ? activeLogName : null;
      const next = keep || (names[0] || null);
      renderLogTabs(files || [], next);
      activeLogName = next;
      if (next) loadActiveLog();
      else $('logContent').textContent = '（暂无日志文件）';
    })
    .catch((e) => toast('读取日志列表失败：' + e, 4000));
}

function showLogs() {
  $('logModal').classList.remove('hidden');
  activeLogName = null;
  refreshLogTabs();
}

function hideLogs() {
  $('logModal').classList.add('hidden');
}

function detectNode() {
  setBusy(true);
  invoke('detect_node')
    .then((info) => {
      $('nodeHint').textContent = info.ok ? '检测结果：' + info.path + '  ' + info.version : info.reason;
      if (info.ok) {
        toast('已检测到 node');
      }
    })
    .catch((e) => toast('检测失败：' + e, 4000))
    .finally(() => setBusy(false));
}

function saveSettings() {
  const portRaw = $('setPort').value.trim();
  const port = Number(portRaw);
  if (!/^\d+$/.test(portRaw) || port < 1024 || port > 65535) {
    toast('端口需为 1024–65535 的整数，当前输入：' + (portRaw || '（空）'), 5000);
    return;
  }
  const profile = $('setProfile').value.trim();
  const settings = {
    port,
    profile: profile || 'web',
  };
  setBusy(true);
  invoke('save_settings', { settings })
    .then(() => {
      toast('设置已保存（重启内核后生效）');
      return refreshAll();
    })
    .catch((e) => toast('保存失败：' + e, 5000))
    .finally(() => setBusy(false));
}

// --- wiring -----------------------------------------------------------------

$('btnRefresh').addEventListener('click', checkUpdates);
$('btnToggle').addEventListener('click', () => {
  const k = currentView && currentView.kernel;
  if (k && k.running) {
    stopWorkbench();
  } else {
    startWorkbench();
  }
});
$('btnOpenWindow').addEventListener('click', openHarnessWindow);
$('btnLogs').addEventListener('click', showLogs);
$('btnLogClose').addEventListener('click', hideLogs);
$('btnLogRefresh').addEventListener('click', () => loadActiveLog());
$('btnConfirmOk').addEventListener('click', () => settleConfirm(true));
$('btnConfirmCancel').addEventListener('click', () => settleConfirm(false));
$('btnDetectNode').addEventListener('click', detectNode);
$('btnSaveSettings').addEventListener('click', saveSettings);
$('btnProgressClose').addEventListener('click', closeProgress);
$('btnShellCheck').addEventListener('click', () => checkShellUpdate(true));
$('btnShellInstall').addEventListener('click', installShellUpdate);
$('btnOpenDataDir').addEventListener('click', openDataDir);

// Startup self-update discovery: the shell emits this after its background
// check; the manual button covers on-demand checks.
if (window.__TAURI__ && window.__TAURI__.event) {
  window.__TAURI__.event.listen('shell-update-available', (e) => {
    showShellUpdateBanner(e.payload);
  });
}

// --- menu navigation --------------------------------------------------------
//
// The sidebar menu switches between panels by toggling .hidden; all panels
// stay in the DOM so their element ids keep working for render functions.

document.querySelectorAll('.menu-item').forEach((item) => {
  item.addEventListener('click', () => {
    document.querySelectorAll('.menu-item').forEach((m) => {
      m.classList.toggle('active', m === item);
    });
    document.querySelectorAll('.panel').forEach((p) => {
      p.classList.toggle('hidden', p.id !== item.dataset.panel);
    });
  });
});

// Status auto-refresh while a kernel may be coming up. Hidden windows skip
// the poll entirely; becoming visible again refreshes immediately.
function pollStatus() {
  if (!core || document.hidden) {
    return;
  }
  invoke('get_status')
    .then((view) => {
      const changed = view.kernel.running !== lastRunning;
      lastRunning = view.kernel.running;
      renderStatus(view);
      // 启动编排自己会 toast「工作台已启动」，这里只提示外部来源的就绪。
      if (changed && view.kernel.running && !starting) {
        toast('内核已就绪', 2500);
      }
    })
    .catch(() => {
      // 后台轮询读状态失败不打扰用户：面板保留旧值，下个周期自动重试。
    });
}

setInterval(pollStatus, 2500);

document.addEventListener('visibilitychange', () => {
  if (!document.hidden) {
    pollStatus();
  }
});

refreshAll();
