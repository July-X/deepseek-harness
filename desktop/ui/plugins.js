'use strict';

// Plugin management card: renders the central store, drives
// install/update/uninstall/sync, and surfaces update reminders.
// Loaded after app.js; reuses its helpers ($, invoke, toast, setBusy,
// setProgress, hideProgress, resetInstallLog, appendInstallLog, installFailed).

let pluginView = null;
let catalogItems = [];

// --- plugin center (community catalog) ------------------------------------

// dsh-plugin.org category ids → 中文标签，顺序即界面顺序（同示意图）。
const CATALOG_CATEGORIES = [
  ['interface', '界面体验'],
  ['session', '会话消息'],
  ['memory', '记忆上下文'],
  ['tools', '工具能力'],
  ['agent', '技能智能体'],
  ['workflow', '工作流'],
  ['integration', '集成连接'],
  ['model', '模型推理'],
  ['dev', '开发运维'],
  ['knowledge', '数据知识'],
  ['fun', '娱乐'],
];
const CATALOG_PAGE = 60;

let catalogLoaded = false;
let catalogCategory = 'all';
let catalogShown = CATALOG_PAGE;

function categoryLabel(id) {
  const hit = CATALOG_CATEGORIES.find(([key]) => key === id);
  return hit ? hit[1] : id || '未分类';
}

function formatCount(n) {
  return n >= 1000 ? (n / 1000).toFixed(1).replace(/\.0$/, '') + 'k' : String(n);
}

function formatUpdated(iso) {
  const t = Date.parse(iso || '');
  if (!t) return '';
  const days = Math.floor((Date.now() - t) / 86400000);
  if (days <= 0) return '今天更新';
  if (days === 1) return '昨天更新';
  if (days < 30) return days + ' 天前更新';
  return '更新于 ' + new Date(t).toISOString().slice(0, 10);
}

// 已安装判定：store 名（git 时为仓库末段）或 repo 全名，均小写比较。
function installedKeys() {
  const keys = new Set();
  const rows = (pluginView && pluginView.rows) || [];
  rows.forEach((row) => {
    keys.add(String(row.name || '').toLowerCase());
    if (row.repo_url) {
      keys.add(row.repo_url.replace(/^https?:\/\/github\.com\//i, '').replace(/\.git$/, '').toLowerCase());
    }
  });
  return keys;
}

function isInstalled(item) {
  const keys = installedKeys();
  if (keys.has(String(item.name || '').toLowerCase())) return true;
  return item.repo ? keys.has(item.repo.toLowerCase()) : false;
}

function filteredCatalog() {
  const q = $('catalogQuery').value.trim().toLowerCase();
  const filter = $('catalogFilter').value;
  const sort = $('catalogSort').value;
  let items = catalogItems.filter((item) => {
    if (catalogCategory !== 'all' && item.category !== catalogCategory) return false;
    if (filter === 'installed' && !isInstalled(item)) return false;
    if (filter === 'not-installed' && isInstalled(item)) return false;
    if (!q) return true;
    const hay = [item.name, item.description, item.repo, item.category, categoryLabel(item.category)]
      .concat(item.tags || [])
      .join(' ')
      .toLowerCase();
    return hay.includes(q);
  });
  if (sort === 'updated') {
    items = items.slice().sort((a, b) => Date.parse(b.updated || '') - Date.parse(a.updated || ''));
  }
  return items;
}

function renderCatalogCats() {
  const box = $('catalogCats');
  box.innerHTML = '';
  const counts = new Map();
  catalogItems.forEach((item) => counts.set(item.category, (counts.get(item.category) || 0) + 1));
  const mkChip = (id, label, count) => {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'cat-chip' + (catalogCategory === id ? ' active' : '');
    chip.textContent = label;
    if (count) {
      const n = document.createElement('span');
      n.className = 'cat-count';
      n.textContent = String(count);
      chip.appendChild(n);
    }
    chip.addEventListener('click', () => {
      catalogCategory = id;
      catalogShown = CATALOG_PAGE;
      renderCatalogCats();
      renderCatalog();
    });
    box.appendChild(chip);
  };
  mkChip('all', '全部', catalogItems.length);
  CATALOG_CATEGORIES.forEach(([id, label]) => {
    if (counts.get(id)) mkChip(id, label, counts.get(id));
  });
  counts.forEach((count, id) => {
    if (id && !CATALOG_CATEGORIES.some(([key]) => key === id)) mkChip(id, id, count);
  });
}

function renderCatalogCard(item) {
  const card = document.createElement('div');
  card.className = 'catalog-card';

  const head = document.createElement('div');
  head.className = 'catalog-card-head';

  const title = document.createElement('span');
  title.className = 'catalog-title';
  const name = document.createElement('span');
  name.className = 'catalog-name';
  name.textContent = item.name;
  title.appendChild(name);
  if (item.version) {
    const ver = document.createElement('span');
    ver.className = 'badge';
    ver.textContent = item.version;
    title.appendChild(ver);
  }
  if (item.category) {
    const cat = document.createElement('span');
    cat.className = 'badge cat';
    cat.textContent = categoryLabel(item.category);
    title.appendChild(cat);
  }
  if (item.verified) {
    const v = document.createElement('span');
    v.className = 'badge installed';
    v.textContent = '已验证';
    title.appendChild(v);
  }
  head.appendChild(title);

  const stats = document.createElement('span');
  stats.className = 'catalog-stats';
  const parts = [];
  if (item.stars > 0) parts.push('★ ' + formatCount(item.stars));
  if (item.forks > 0) parts.push('Fork ' + formatCount(item.forks));
  const updated = formatUpdated(item.updated);
  if (updated) parts.push(updated);
  stats.textContent = parts.join(' · ');
  head.appendChild(stats);
  card.appendChild(head);

  if (item.description) {
    const desc = document.createElement('p');
    desc.className = 'catalog-desc';
    desc.textContent = item.description.length > 140 ? item.description.slice(0, 137) + '…' : item.description;
    card.appendChild(desc);
  }

  const foot = document.createElement('div');
  foot.className = 'catalog-card-foot';
  const tags = document.createElement('span');
  tags.className = 'catalog-tags';
  (item.tags || []).slice(0, 4).forEach((tag) => {
    const t = document.createElement('span');
    t.className = 'tag';
    t.textContent = tag;
    tags.appendChild(t);
  });
  foot.appendChild(tags);

  const actions = document.createElement('span');
  actions.className = 'catalog-actions';
  const detailUrl = item.detail_url || (item.repo ? 'https://github.com/' + item.repo : '');
  if (detailUrl) {
    const detail = document.createElement('button');
    detail.type = 'button';
    detail.className = 'ghost';
    detail.textContent = '打开详情';
    detail.addEventListener('click', () => openExternal(detailUrl));
    actions.appendChild(detail);
  }
  const installed = isInstalled(item);
  const btn = document.createElement('button');
  btn.type = 'button';
  if (installed) {
    btn.className = 'ghost';
    btn.textContent = '已安装';
    btn.disabled = true;
  } else {
    btn.textContent = '安装';
    btn.addEventListener('click', () => installPlugin(item.spec));
  }
  actions.appendChild(btn);
  foot.appendChild(actions);
  card.appendChild(foot);
  return card;
}

function renderCatalog() {
  const list = $('catalogList');
  list.innerHTML = '';
  if (!catalogLoaded) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = '目录加载中…';
    list.appendChild(p);
    return;
  }
  const items = filteredCatalog();
  const verified = catalogItems.filter((i) => i.verified).length;
  $('catalogCount').textContent = catalogItems.length
    ? '结果 ' + items.length + ' 条 · 收录 ' + catalogItems.length + ' 款 · 已验证 ' + verified + ' 款'
    : '';
  if (items.length === 0) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = catalogItems.length ? '没有匹配的插件，换个关键词或分类试试。' : '目录为空或加载失败，点「刷新目录」重试。';
    list.appendChild(p);
  } else {
    items.slice(0, catalogShown).forEach((item) => list.appendChild(renderCatalogCard(item)));
  }
  $('btnCatalogMore').classList.toggle('hidden', items.length <= catalogShown);
}

function loadCatalog(manual) {
  catalogLoaded = false;
  renderCatalog();
  if (manual) setBusy(true);
  return invoke('plugin_catalog', { force: !!manual })
    .then((items) => {
      catalogItems = items || [];
      catalogLoaded = true;
      catalogShown = CATALOG_PAGE;
      renderCatalogCats();
      renderCatalog();
    })
    .catch((e) => {
      catalogLoaded = true;
      renderCatalog();
      toast('目录加载失败：' + e, 6000);
    })
    .finally(() => {
      if (manual) setBusy(false);
    });
}

// --- rendering ------------------------------------------------------------

function renderUpdateBadge() {
  const badge = $('pluginUpdateBadge');
  const n = pluginView ? pluginView.updates : 0;
  if (n > 0) {
    badge.textContent = n + ' 个更新';
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
    // `installed_version` always carries whatever prefix the source
    // provides; for git that's usually `v<hash>` or `v<tag>`, for npm
    // it's plain semver. When a newer release is known, append the
    // `→ <latest>` arrow so the gap is visible at a glance and the
    // action-area "有更新" badge stops repeating the prefix.
    const installed = (row.origin === 'npm' ? 'npm' : 'git') + ' · ' + row.installed_version;
    const upgrade = row.latest_version ? ' → ' + row.latest_version : '';
    meta.textContent = installed + upgrade + pinNote;
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
      badge.textContent = '有更新 ' + row.latest_version;
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
  const fromCatalog = !!(spec || '').trim();
  const raw = (spec || '').trim() || input.value.trim();
  if (!raw) {
    toast('请先填写 npm 包名或仓库地址', 4000);
    return;
  }
  if (!fromCatalog) {
    input.value = '';
  }
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

function openExternal(url) {
  if (window.__TAURI__ && window.__TAURI__.core) {
    window.__TAURI__.core.invoke('plugin:opener|open_url', { url }).catch(() => {});
  }
}

// --- wiring ---------------------------------------------------------------

$('btnPluginInstall').addEventListener('click', () => installPlugin(''));
$('btnPluginCheck').addEventListener('click', () => checkPluginUpdates(false));
$('btnPluginSync').addEventListener('click', syncPlugins);
$('btnCatalogReload').addEventListener('click', () => loadCatalog(true));
$('btnCatalogMore').addEventListener('click', () => {
  catalogShown += CATALOG_PAGE;
  renderCatalog();
});
let catalogQueryTimer = null;
$('catalogQuery').addEventListener('input', () => {
  clearTimeout(catalogQueryTimer);
  catalogQueryTimer = setTimeout(() => {
    catalogShown = CATALOG_PAGE;
    renderCatalog();
  }, 150);
});
$('catalogSort').addEventListener('change', () => {
  catalogShown = CATALOG_PAGE;
  renderCatalog();
});
$('catalogFilter').addEventListener('change', () => {
  catalogShown = CATALOG_PAGE;
  renderCatalog();
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
      // 安装状态变了，目录卡片的「安装 / 已安装」按钮跟着刷新
      if (catalogLoaded) {
        renderCatalog();
      }
    })
    .catch(() => {});
};

// 启动后预载社区目录（静默，失败不打断：用户可在插件中心手动刷新）
setTimeout(() => {
  loadCatalog(false);
}, 1200);

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
