'use strict';

// Plugin management card: renders the central store, drives
// install/update/uninstall/sync, and surfaces update reminders.
// Loaded after app.js; reuses its helpers ($, invoke, toast, setBusy,
// setProgress, hideProgress, resetInstallLog, appendInstallLog, installFailed,
// el, mkBtn, armConfirm). withPluginProgress is shared back with app.js,
// whose kernel install runs through it.

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

// keys comes from the caller: each render pass builds the Set once via
// installedKeys() instead of re-running the repo_url regexes per item.
function isInstalled(item, keys) {
  if (keys.has(String(item.name || '').toLowerCase())) return true;
  return item.repo ? keys.has(item.repo.toLowerCase()) : false;
}

function filteredCatalog(keys) {
  const q = $('catalogQuery').value.trim().toLowerCase();
  const filter = $('catalogFilter').value;
  const sort = $('catalogSort').value;
  let items = catalogItems.filter((item) => {
    if (catalogCategory !== 'all' && item.category !== catalogCategory) return false;
    if (filter === 'installed' && !isInstalled(item, keys)) return false;
    if (filter === 'not-installed' && isInstalled(item, keys)) return false;
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

function renderCatalogCard(item, keys) {
  const card = el('div', 'catalog-card');
  const head = el('div', 'catalog-card-head');
  const title = el('span', 'catalog-title');
  title.appendChild(el('span', 'catalog-name', item.name));
  if (item.version) {
    title.appendChild(el('span', 'badge', item.version));
  }
  if (item.category) {
    title.appendChild(el('span', 'badge cat', categoryLabel(item.category)));
  }
  if (item.verified) {
    title.appendChild(el('span', 'badge installed', '已验证'));
  }
  head.appendChild(title);

  const parts = [];
  if (item.stars > 0) parts.push('★ ' + formatCount(item.stars));
  if (item.forks > 0) parts.push('Fork ' + formatCount(item.forks));
  const updated = formatUpdated(item.updated);
  if (updated) parts.push(updated);
  head.appendChild(el('span', 'catalog-stats', parts.join(' · ')));
  card.appendChild(head);

  if (item.description) {
    card.appendChild(el('p', 'catalog-desc',
      item.description.length > 140 ? item.description.slice(0, 137) + '…' : item.description));
  }

  const foot = el('div', 'catalog-card-foot');
  const tags = el('span', 'catalog-tags');
  (item.tags || []).slice(0, 4).forEach((tag) => {
    tags.appendChild(el('span', 'tag', tag));
  });
  foot.appendChild(tags);

  const actions = el('span', 'catalog-actions');
  const detailUrl = item.detail_url || (item.repo ? 'https://github.com/' + item.repo : '');
  if (detailUrl) {
    actions.appendChild(mkBtn('打开详情', () => openExternal(detailUrl), 'ghost'));
  }
  if (isInstalled(item, keys)) {
    const btn = el('button', 'ghost', '已安装');
    btn.type = 'button';
    btn.disabled = true;
    actions.appendChild(btn);
  } else {
    actions.appendChild(mkBtn('安装', () => installPlugin(item.spec)));
  }
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
  const keys = installedKeys();
  const items = filteredCatalog(keys);
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
    items.slice(0, catalogShown).forEach((item) => list.appendChild(renderCatalogCard(item, keys)));
  }
  $('btnCatalogMore').classList.toggle('hidden', items.length <= catalogShown);
}

function loadCatalog(manual) {
  catalogLoaded = false;
  renderCatalog();
  // The catalog fetch is a pure network read — it does not mutate any
  // shared state, hold the kernel, or take a lock the rest of the UI
  // needs. Guard only the refresh button itself so a double-click does
  // not queue two `plugin_catalog` invokes back-to-back; leave every
  // other control (sidebar, kernel toggle, installed-plugin actions,
  // catalog search / sort / filter / pagination) interactive so the
  // user can keep working while the fetch is in flight. The previous
  // `setBusy(true)` here disabled the entire shell, and the
  // catalog-render pass that follows destroyed+recreated catalog
  // buttons whose `disabled` state lived in the `busyButtons` Set
  // managed by setBusy — the recreation left the new buttons in the
  // Set's bookkeeping for the next `setBusy(false)` to re-enable, but
  // the round trip was fragile (any other setBusy(true)/(false) in
  // between could strand a button permanently disabled).
  const reload = $('btnCatalogReload');
  const more = $('btnCatalogMore');
  if (manual) {
    reload.disabled = true;
    if (more) more.disabled = true;
  }
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
      if (manual) {
        reload.disabled = false;
        if (more) more.disabled = items && items.length > catalogShown ? false : true;
      }
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
    list.appendChild(el('p', 'muted', '尚未安装任何插件。'));
    return;
  }
  view.rows.forEach((row) => {
    const item = el('div', 'installed-row plugin-row');
    const info = el('span', 'plugin-info');
    info.appendChild(el('span', 'release-ver', row.name));

    const pinNote = row.pinned ? ' · 已锁定版本' : '';
    // `installed_version` always carries whatever prefix the source
    // provides; for git that's usually `v<hash>` or `v<tag>`, for npm
    // it's plain semver. When a newer release is known, append the
    // `→ <latest>` arrow so the gap is visible at a glance and the
    // action-area "有更新" badge stops repeating the prefix.
    const installed = (row.origin === 'npm' ? 'npm' : 'git') + ' · ' + row.installed_version;
    const upgrade = row.latest_version ? ' → ' + row.latest_version : '';
    info.appendChild(el('span', 'plugin-meta', installed + upgrade + pinNote));
    if (row.quarantined) {
      // 启动看护的隔离原因就地展示，用户不必回到事故面板才知道这个插件
      // 为什么没有生效。
      const reason = String(row.quarantined.reason || '');
      info.appendChild(el('span', 'plugin-meta quarantine-note',
        '已隔离：' + (reason.length > 60 ? reason.slice(0, 57) + '…' : reason)));
    }
    item.appendChild(info);

    const actions = el('span', 'release-actions plugin-actions');

    if (row.actual_mode === 'copy') {
      actions.appendChild(el('span', 'badge', '复制'));
    } else if (row.actual_mode === 'link') {
      actions.appendChild(el('span', 'badge installed', '链接'));
    }
    if (row.synced && row.wired) {
      actions.appendChild(el('span', 'badge installed', '已同步'));
    } else if (pluginView && pluginView.active_kernel && !row.synced) {
      actions.appendChild(el('span', 'badge warn', '待同步'));
    }
    if (!row.wired && pluginView && pluginView.active_kernel) {
      actions.appendChild(el('span', 'badge warn', '待接线'));
    }
    if (!pluginView || !pluginView.active_kernel) {
      actions.appendChild(el('span', 'badge warn', '无活动内核'));
    }
    if (row.quarantined) {
      actions.appendChild(el('span', 'badge warn', '已停用'));
      actions.appendChild(mkBtn('恢复启用', () => resolvePluginQuarantine(row.id, 'enable'), 'ghost'));
    }
    if (row.latest_version) {
      actions.appendChild(el('span', 'badge update', '有更新 ' + row.latest_version));
      if (!row.pinned) {
        actions.appendChild(mkBtn('更新', () => updatePlugin(row.id)));
      }
    }
    actions.appendChild(mkBtn(
      row.desired_mode === 'copy' ? '切换为链接' : '切换为复制',
      () => setPluginMode(row.id, row.desired_mode === 'copy' ? 'link' : 'copy'),
      'ghost'
    ));
    if (row.repo_url) {
      actions.appendChild(mkBtn('仓库', () => openExternal(row.repo_url), 'ghost'));
    }
    actions.appendChild(mkBtn('卸载', (ev) => armConfirm(ev.currentTarget, {
      armedLabel: '确认卸载？',
      idleLabel: '卸载',
      onConfirm: (btn) => proceedPluginRemove(row.id, btn),
    }), 'danger'));

    item.appendChild(actions);
    list.appendChild(item);
  });
}

// --- actions --------------------------------------------------------------

function withPluginProgress(labels, task) {
  const channel = new core.Channel();
  channel.onmessage = (msg) => {
    // Stage messages from the install commands start with a CJK sentence;
    // raw pnpm log lines arrive verbatim and go to the scrolling log area.
    appendInstallLog(msg);
    setProgress(msg.length > 60 ? msg.slice(0, 57) + '…' : msg);
  };
  setBusy(true);
  resetInstallLog();
  setProgress(labels.start);
  return invoke(labels.cmd, task(channel))
    .then(() => {
      // Callers like updatePlugin show their own success toast afterwards.
      if (labels.done) {
        toast(labels.done);
      }
      return refreshAll();
    })
    .catch((e) => {
      const failLabel = labels.fail || '操作失败';
      installFailed = true;
      setProgress(failLabel + '：' + e);
      $('progressActions').classList.remove('hidden');
      appendInstallLog('—— ' + failLabel + '：' + e + ' ——');
      toast(labels.failToast || '操作失败，详情见进度窗口与日志', 6000);
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
    toast('请先填写仓库地址或 npm 包名', 4000);
    return;
  }
  if (!fromCatalog) {
    input.value = '';
  }
  // 物化模式默认走链接（plugin_install 的 mode 缺省回退到 link）；
  // 用户可在「已安装」列表的「切换为复制 / 切换为链接」按钮上调整
  // （setPluginMode → plugin_set_mode），那里才是模式权威入口。
  return withPluginProgress(
    {
      cmd: 'plugin_install',
      start: '正在安装插件 ' + raw + ' …',
      done: '插件 ' + raw + ' 已安装（重启内核后生效）',
      fail: '安装失败：' + raw
    },
    (channel) => ({ spec: raw, onEvent: channel })
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

// 恢复启用（清除隔离记录并重新接线）或直接卸载被隔离的插件。
// 与事故面板共用同一个 plugin_resolve 命令；恢复后需重启工作台生效。
function resolvePluginQuarantine(id, action) {
  return withPluginProgress(
    {
      cmd: 'plugin_resolve',
      start: action === 'remove' ? '正在卸载插件 …' : '正在恢复插件接线 …',
      done: action === 'remove' ? '插件已移除' : '插件已恢复，重启工作台后生效',
      fail: action === 'remove' ? '卸载失败' : '恢复失败'
    },
    (channel) => ({ id, action, onEvent: channel })
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

// Manual checks pass { busy: true, toastOnUpdates: true }; the startup
// self-check stays silent on errors and skips the busy lock.
function checkPluginUpdates(opts) {
  if (opts.busy) {
    setBusy(true);
  }
  return invoke('plugin_check_updates')
    .then((infos) => {
      const n = (infos || []).filter((i) => i.latest).length;
      if (n > 0 && opts.toastOnUpdates) {
        toast('有 ' + n + ' 个插件可更新', 5000);
      }
      return refreshAll();
    })
    .catch((e) => {
      // 静默路径（启动自检）失败不打扰用户，只有手动检查才弹错误。
      if (opts.busy) {
        toast('检查插件更新失败：' + e, 6000);
      }
    })
    .finally(() => {
      if (opts.busy) {
        setBusy(false);
      }
    });
}

function openExternal(url) {
  if (window.__TAURI__ && window.__TAURI__.core) {
    window.__TAURI__.core.invoke('plugin:opener|open_url', { url }).catch(() => {});
  }
}

// --- wiring ---------------------------------------------------------------

$('btnPluginCheck').addEventListener('click', () => checkPluginUpdates({ busy: true, toastOnUpdates: true }));
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
    .catch(() => {
      // 静默刷新：插件状态读取失败时保留旧卡片，下次 refreshAll 再试。
    });
};

// 启动后预载社区目录（静默，失败不打断：用户可在插件中心手动刷新）
setTimeout(() => {
  loadCatalog(false);
}, 1200);

// 启动后静默检查一次插件更新，有新版时提醒
setTimeout(() => {
  checkPluginUpdates({ busy: false, toastOnUpdates: true });
}, 3500);
