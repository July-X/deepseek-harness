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

// --- toast ---------------------------------------------------------------

let toastTimer = null;
function toast(msg, ms) {
  const el = $('toast');
  el.textContent = msg;
  el.classList.remove('hidden');
  if (toastTimer) {
    clearTimeout(toastTimer);
  }
  toastTimer = setTimeout(() => el.classList.add('hidden'), ms || 3200);
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
  document.querySelectorAll('button').forEach((btn) => {
    if (on) {
      if (!btn.dataset.wasDisabled) {
        btn.dataset.wasDisabled = String(btn.disabled);
      }
      btn.disabled = true;
      busyButtons.add(btn);
    } else if (busyButtons.has(btn)) {
      btn.disabled = btn.dataset.wasDisabled === 'true';
      busyButtons.delete(btn);
    }
  });
}

// --- rendering -------------------------------------------------------------

let releases = [];
let currentView = null;

function renderStatus(view) {
  currentView = view;
  const { kernel, node, settings } = view;

  const pill = $('statusPill');
  const dot = $('statusDot');
  const text = $('statusText');
  if (kernel.running) {
    pill.classList.remove('hidden');
    dot.className = 'dot ok';
    text.textContent = '运行中';
  } else if (kernel.active && kernel.active_installed) {
    dot.className = 'dot bad';
    text.textContent = '已停止';
  } else {
    dot.className = '';
    text.textContent = '未安装';
  }

  $('kernelRunning').textContent = kernel.running ? '运行中' : '未运行';
  $('kernelActive').textContent = kernel.active || '（未选择）';
  $('kernelUrl').textContent = kernel.running ? 'http://127.0.0.1:' + kernel.port : '—';
  $('kernelNode').textContent = node.ok
    ? [node.path, node.version].filter(Boolean).join('  ')
    : '未检测到可用 Node（' + node.reason + '）';
  $('kernelHome').textContent = kernel.dsh_home;

  $('updateActive').textContent = kernel.active || '（未选择）';
  $('updateRunning').textContent = kernel.running ? '运行中' : '未运行';
  $('updateInstalled').textContent = String((kernel.installed || []).length) + ' 个';

  $('setPort').value = String(settings.port);
  $('setProfile').value = settings.profile || '';

  $('nodeHint').textContent = node.ok
    ? 'node ' + node.version + ' 满足 dsh 要求（^22.19 || >=24）'
    : node.reason;
}

function badgeFor(version) {
  const k = currentView && currentView.kernel;
  if (k && k.active === version) {
    return '<span class="badge active">当前使用</span>';
  }
  if (installedVersions().has(version)) {
    return '<span class="badge installed">已安装</span>';
  }
  return '';
}

// Two-step confirmation for destructive actions: WKWebView does not support
// window.confirm, so removal uses an in-page armed state instead.
let pendingRemove = null;
let pendingRemoveTimer = null;

function armRemove(version, btn) {
  if (pendingRemove === version) {
    clearTimeout(pendingRemoveTimer);
    pendingRemove = null;
    btn.classList.remove('armed');
    proceedRemove(version, btn);
    return;
  }
  pendingRemove = version;
  btn.textContent = '确认删除？';
  btn.classList.add('armed');
  pendingRemoveTimer = setTimeout(() => {
    btn.textContent = '删除';
    btn.classList.remove('armed');
    if (pendingRemove === version) {
      pendingRemove = null;
    }
  }, 3200);
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
  releases.forEach((r) => {
    const row = document.createElement('div');
    row.className = 'release-row';

    const ver = document.createElement('span');
    ver.className = 'release-ver';
    ver.textContent = r.version;

    const actions = document.createElement('span');
    actions.className = 'release-actions';
    actions.innerHTML = badgeFor(r.version);

    const k = currentView && currentView.kernel;
    const installed = installedVersions().has(r.version);
    const isActive = k && k.active === r.version;

    if (!installed) {
      const installBtn = document.createElement('button');
      installBtn.type = 'button';
      installBtn.textContent = '安装';
      installBtn.addEventListener('click', () => installVersion(r.version));
      actions.appendChild(installBtn);
    } else if (!isActive) {
      const activeBtn = document.createElement('button');
      activeBtn.type = 'button';
      activeBtn.textContent = '切换';
      activeBtn.addEventListener('click', () => activateVersion(r.version));
      actions.appendChild(activeBtn);
    }

    if (r.prerelease) {
      const pre = document.createElement('span');
      pre.className = 'badge';
      pre.textContent = '预发布';
      actions.appendChild(pre);
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
    const row = document.createElement('div');
    row.className = 'installed-row';

    const ver = document.createElement('span');
    ver.className = 'release-ver';
    ver.textContent = v.version;

    const actions = document.createElement('span');
    actions.className = 'release-actions';
    if (v.active) {
      const badge = document.createElement('span');
      badge.className = 'badge active';
      badge.textContent = '当前使用';
      actions.appendChild(badge);
    }
    if (!v.active) {
      const rm = document.createElement('button');
      rm.type = 'button';
      rm.className = 'danger';
      rm.textContent = '删除';
      rm.addEventListener('click', (ev) => armRemove(v.version, ev.currentTarget));
      actions.appendChild(rm);
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

function installVersion(version) {
  const channel = new core.Channel();
  channel.onmessage = (msg) => {
    // Stage messages from install_version start with a CJK sentence; raw
    // pnpm log lines arrive verbatim and go to the scrolling log area.
    appendInstallLog(msg);
    setProgress(msg.length > 60 ? msg.slice(0, 57) + '…' : msg);
  };
  setBusy(true);
  resetInstallLog();
  setProgress('正在安装 ' + version + ' …');
  invoke('install_kernel', { version, onEvent: channel })
    .then(() => {
      toast('版本 ' + version + ' 安装完成');
      return refreshAll();
    })
    .catch((e) => {
      // Keep the overlay open so the user can read the full log; the close
      // button appears and the app stays usable after it is clicked.
      installFailed = true;
      setProgress('安装失败：' + e);
      $('progressActions').classList.remove('hidden');
      appendInstallLog('—— 安装失败：' + e + ' ——');
      toast('安装失败，详情见进度窗口与日志', 6000);
      return refreshAll();
    })
    .finally(() => {
      setBusy(false);
      if (!installFailed) {
        hideProgress();
      }
    });
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

function startKernel() {
  setBusy(true);
  invoke('start_kernel')
    .then(() => {
      toast('内核已启动');
      return refreshAll();
    })
    .catch((e) => toast('启动失败：' + e, 6000))
    .finally(() => setBusy(false));
}

function stopKernel() {
  setBusy(true);
  invoke('stop_kernel')
    .then(() => {
      toast('内核已停止');
      return refreshAll();
    })
    .catch((e) => toast('停止失败：' + e, 5000))
    .finally(() => setBusy(false));
}

function openHarness() {
  invoke('open_harness')
    .then(() => {})
    .catch((e) => toast('无法打开工作台：' + e, 5000));
}

function showLogs() {
  invoke('get_kernel_log')
    .then((text) => {
      $('logContent').textContent = text || '（暂无日志）';
      $('logModal').classList.remove('hidden');
    })
    .catch((e) => toast('读取日志失败：' + e, 4000));
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
  const settings = {
    port: parseInt($('setPort').value, 10) || 3080,
    profile: $('setProfile').value.trim() || 'web',
  };
  setBusy(true);
  invoke('save_settings', { settings })
    .then(() => {
      toast('设置已保存');
      return refreshAll();
    })
    .catch((e) => toast('保存失败：' + e, 5000))
    .finally(() => setBusy(false));
}

// --- wiring -----------------------------------------------------------------

$('btnRefresh').addEventListener('click', checkUpdates);
$('btnStart').addEventListener('click', startKernel);
$('btnStop').addEventListener('click', stopKernel);
$('btnOpen').addEventListener('click', openHarness);
$('btnLogs').addEventListener('click', showLogs);
$('btnLogClose').addEventListener('click', hideLogs);
$('btnDetectNode').addEventListener('click', detectNode);
$('btnSaveSettings').addEventListener('click', saveSettings);
$('btnProgressClose').addEventListener('click', closeProgress);

// Status auto-refresh while a kernel may be coming up.
let lastRunning = false;
setInterval(() => {
  if (!core) {
    return;
  }
  invoke('get_status')
    .then((view) => {
      const changed = view.kernel.running !== lastRunning;
      lastRunning = view.kernel.running;
      renderStatus(view);
      if (changed && view.kernel.running) {
        toast('内核已就绪', 2500);
      }
    })
    .catch(() => {});
}, 2500);

refreshAll().catch(() => {});
