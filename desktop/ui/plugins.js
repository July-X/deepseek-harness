'use strict';

// Plugin management card: renders the central store, drives
// install/update/uninstall/sync, and surfaces update reminders.
// Loaded after app.js; reuses its helpers ($, invoke, toast, setBusy,
// setProgress, hideProgress, resetInstallLog, appendInstallLog, installFailed).

let pluginView = null;
let catalogItems = [];

// --- rendering ------------------------------------------------------------

function renderUpdateBadge() {
  const badge = $('pluginUpdateBadge');
  const n = pluginView ? pluginView.updates : 0;
  if (n > 0) {
    badge.textContent = n + ' 个更新可用';
    badge.classList.remove('hidden');
  } else {
    badge.classList.add('hidden');
  }
}

function renderWarning() {
  const warn = $('pluginWarning');
  if (pluginView && pluginView.warning) {
    warn.textContent = pluginView.warning + '（可在「日志」侧查看 plugin-wiring.log）';
    warn.classList.remove('hidden');
  } else {
    warn.classList.add('hidden');
  }
}

function renderPluginList() {
  const list = $('pluginList');
  list.innerHTML = '';
  const view = pluginView;
  if (!view || !view.rows || view.rows.length === 0) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = '尚未安装任何插件。';
    list.appendChild(p);
    return;
  }
  view.rows.forEach((row) => {
    const item = document.createElement('div');
    item.className = 'installed-row plugin-row';

    const info = document.createElement('span');
    info.className = 'plugin-info';

    const name = document.createElement('span');
    name.className = 'release-ver';
    name.textContent = row.name;
    info.appendChild(name);

    const meta = document.createElement('span');
    meta.className = 'plugin-meta';
    const pinNote = row.pinned ? ' · 已锁定版本' : '';
    meta.textContent = (row.origin === 'npm' ? 'npm' : 'git') + ' · v' + row.installed_version + pinNote;
    info.appendChild(meta);
    item.appendChild(info);

    const actions = document.createElement('span');
    actions.className = 'release-actions plugin-actions';

    if (row.actual_mode === 'copy') {
      const badge = document.createElement('span');
      badge.className = 'badge';
      badge.textContent = '复制';
      actions.appendChild(badge);
    } else if (row.actual_mode === 'link') {
      const badge = document.createElement('span');
      badge.className = 'badge installed';
      badge.textContent = '链接';
      actions.appendChild(badge);
    }
    if (row.synced && row.wired) {
      const badge = document.createElement('span');
      badge.className = 'badge installed';
      badge.textContent = '已同步';
      actions.appendChild(badge);
    } else if (pluginView && pluginView.active_kernel && !row.synced) {
      const badge = document.createElement('span');
      badge.className = 'badge warn';
      badge.textContent = '待同步';
      actions.appendChild(badge);
    }
    if (!row.wired && pluginView && pluginView.active_kernel) {
      const badge = document.createElement('span');
      badge.className = 'badge warn';
      badge.textContent = '待接线';
      actions.appendChild(badge);
    }
    if (!pluginView || !pluginView.active_kernel) {
      const badge = document.createElement('span');
      badge.className = 'badge warn';
      badge.textContent = '无活动内核';
      actions.appendChild(badge);
    }
    if (row.latest_version) {
      const badge = document.createElement('span');
      badge.className = 'badge update';
      badge.textContent = '有更新 v' + row.latest_version;
      actions.appendChild(badge);
      if (!row.pinned) {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.textContent = '更新';
        btn.addEventListener('click', () => updatePlugin(row.id));
        actions.appendChild(btn);
      }
    }
    const mode = document.createElement('button');
    mode.type = 'button';
    mode.className = 'ghost';
    mode.textContent = row.desired_mode === 'copy' ? '切换为链接' : '切换为复制';
    mode.addEventListener('click', () => setPluginMode(row.id, row.desired_mode === 'copy' ? 'link' : 'copy'));
    actions.appendChild(mode);
    if (row.repo_url) {
      const repo = document.createElement('button');
      repo.type = 'button';
      repo.className = 'ghost';
      repo.textContent = '仓库';
      repo.addEventListener('click', () => openExternal(row.repo_url));
      actions.appendChild(repo);
    }
    const rm = document.createElement('button');
    rm.type = 'button';
    rm.className = 'danger';
    rm.textContent = '卸载';
    rm.addEventListener('click', (ev) => armPluginRemove(row.id, ev.currentTarget));
    actions.appendChild(rm);

    item.appendChild(actions);
    list.appendChild(item);
  });
}

function renderCatalog() {
  const list = $('catalogList');
  list.innerHTML = '';
  if (catalogItems.length === 0) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = '没有匹配的社区插件。';
    list.appendChild(p);
    return;
  }
  catalogItems.forEach((item) => {
    const row = document.createElement('div');
    row.className = 'release-row catalog-row';

    const info = document.createElement('span');
    info.className = 'catalog-info';
    const name = document.createElement('span');
    name.className = 'release-ver';
    name.textContent = item.name;
    info.appendChild(name);
    const desc = document.createElement('span');
    desc.className = 'plugin-meta';
    const extra = [item.kind, item.category, item.stars > 0 ? item.stars + ' stars' : ''].filter(Boolean).join(' · ');
    const note = item.verified ? ' · 已验证' : ' · 未验证';
    desc.textContent = (extra || '社区条目') + note;
    info.appendChild(desc);
    if (item.description) {
      const brief = document.createElement('span');
      brief.className = 'catalog-desc';
      brief.textContent = item.description.length > 140 ? item.description.slice(0, 137) + '…' : item.description;
      info.appendChild(brief);
    }
    row.appendChild(info);

    const actions = document.createElement('span');
    actions.className = 'release-actions';
    const badge = document.createElement('span');
    badge.className = 'badge';
    badge.textContent = item.origin;
    actions.appendChild(badge);
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = '安装';
    btn.addEventListener('click', () => installPlugin(item.spec));
    actions.appendChild(btn);
    row.appendChild(actions);
    list.appendChild(row);
  });
}

// --- actions --------------------------------------------------------------

function withPluginProgress(labels, task) {
  const channel = new core.Channel();
  channel.onmessage = (msg) => {
    appendInstallLog(msg);
    setProgress(msg.length > 60 ? msg.slice(0, 57) + '…' : msg);
  };
  setBusy(true);
  resetInstallLog();
  setProgress(labels.start);
  return invoke(labels.cmd, task(channel))
    .then(() => {
      toast(labels.done);
      return refreshAll();
    })
    .catch((e) => {
      installFailed = true;
      setProgress(labels.fail + '：' + e);
      $('progressActions').classList.remove('hidden');
      appendInstallLog('—— ' + labels.fail + '：' + e + ' ——');
      toast('操作失败，详情见进度窗口与日志', 6000);
      return refreshAll();
    })
    .finally(() => {
      setBusy(false);
      if (!installFailed) {
        hideProgress();
      }
    });
}

function installPlugin(spec) {
  const input = $('pluginSpec');
  const raw = (spec || '').trim() || input.value.trim();
  if (!raw) {
    toast('请先填写 npm 包名或仓库地址', 4000);
    return;
  }
  input.value = '';
  const mode = $('pluginMode').value;
  return withPluginProgress(
    {
      cmd: 'plugin_install',
      start: '正在安装插件 ' + raw + ' …',
      done: '插件 ' + raw + ' 已安装（重启内核后生效）',
      fail: '安装失败：' + raw
    },
    (channel) => ({ spec: raw, mode, onEvent: channel })
  );
}

function updatePlugin(id) {
  return withPluginProgress(
    {
      cmd: 'plugin_update',
      start: '正在更新插件 …'
    },
    (channel) => ({ id, onEvent: channel })
  ).then((ok) => {
    if (ok) {
      toast('插件已更新，重启内核后生效');
    }
  });
}

function setPluginMode(id, mode) {
  const label = mode === 'copy' ? '复制' : '链接';
  return withPluginProgress(
    {
      cmd: 'plugin_set_mode',
      start: '正在切换为' + label + '模式 …',
      done: '已切换为' + label + '模式'
    },
    (channel) => ({ id, mode, onEvent: channel })
  );
}

function syncPlugins() {
  return withPluginProgress(
    {
      cmd: 'plugin_sync',
      start: '正在同步插件到所有内核 …',
      done: '插件已同步'
    },
    (channel) => ({ onEvent: channel })
  );
}

let pendingPluginRemove = null;
let pendingPluginRemoveTimer = null;

function armPluginRemove(id, btn) {
  if (pendingPluginRemove === id) {
    clearTimeout(pendingPluginRemoveTimer);
    pendingPluginRemove = null;
    btn.classList.remove('armed');
    proceedPluginRemove(id, btn);
    return;
  }
  pendingPluginRemove = id;
  btn.textContent = '确认卸载？';
  btn.classList.add('armed');
  pendingPluginRemoveTimer = setTimeout(() => {
    btn.textContent = '卸载';
    btn.classList.remove('armed');
    if (pendingPluginRemove === id) {
      pendingPluginRemove = null;
    }
  }, 3200);
}

function proceedPluginRemove(id, btn) {
  btn.disabled = true;
  return withPluginProgress(
    {
      cmd: 'plugin_uninstall',
      start: '正在卸载插件 …',
      done: '插件已卸载',
      fail: '卸载失败'
    },
    (channel) => ({ id, onEvent: channel })
  );
}

function checkPluginUpdates(silent) {
  setBusy(true);
  return invoke('plugin_check_updates')
    .then((infos) => {
      const n = (infos || []).filter((i) => i.latest).length;
      if (n > 0 && !silent) {
        toast('有 ' + n + ' 个插件可更新', 5000);
      }
      return refreshAll();
    })
    .catch((e) => {
      if (!silent) {
        toast('检查插件更新失败：' + e, 6000);
      }
    })
    .finally(() => setBusy(false));
}

function searchCatalog() {
  const q = $('catalogQuery').value.trim();
  setBusy(true);
  return invoke('plugin_catalog', { query: q })
    .then((items) => {
      catalogItems = items || [];
      renderCatalog();
      if (catalogItems.length === 0) {
        toast('没有找到匹配的社区插件', 4000);
      }
    })
    .catch((e) => toast('目录搜索失败：' + e, 6000))
    .finally(() => setBusy(false));
}

function openExternal(url) {
  if (window.__TAURI__ && window.__TAURI__.core) {
    window.__TAURI__.core.invoke('plugin:opener|open_url', { url }).catch(() => {});
  }
}

// --- wiring ---------------------------------------------------------------

$('btnPluginInstall').addEventListener('click', () => installPlugin(''));
$('btnPluginCheck').addEventListener('click', () => checkPluginUpdates(false));
$('btnPluginSync').addEventListener('click', syncPlugins);
$('btnCatalogSearch').addEventListener('click', searchCatalog);
document.getElementById('catalogQuery').addEventListener('keydown', (ev) => {
  if (ev.key === 'Enter') {
    searchCatalog();
  }
});
$('pluginSpec').addEventListener('keydown', (ev) => {
  if (ev.key === 'Enter') {
    installPlugin('');
  }
});

// app.js 的 refreshAll 会调用这个钩子，与内核状态一起刷新插件卡片
window.__dshPluginsRefresh = () => {
  return invoke('plugin_status')
    .then((view) => {
      pluginView = view;
      renderUpdateBadge();
      renderWarning();
      renderPluginList();
    })
    .catch(() => {});
};

// 启动后静默检查一次插件更新，有新版时提醒
setTimeout(() => {
  invoke('plugin_check_updates')
    .then((infos) => {
      const n = (infos || []).filter((i) => i.latest).length;
      if (n > 0) {
        toast('有 ' + n + ' 个插件可更新', 6000);
      }
      return window.__dshPluginsRefresh ? window.__dshPluginsRefresh() : null;
    })
    .catch(() => {});
}, 3500);
