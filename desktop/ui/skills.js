'use strict';

// Skill management card: renders the central store (one row per package,
// one chip per skill), drives install/update/uninstall/enable/disable, and
// surfaces update reminders.
// Loaded after app.js and plugins.js; reuses their helpers ($, invoke,
// toast, el, mkBtn, openExternal) plus withPluginProgress from plugins.js
// so every long task shares the same progress panel and log stream.

let skillView = null;
let skillCatalogItems = [];
let skillCatalogLoaded = false;
const SKILL_CATALOG_PAGE = 60;
let skillCatalogShown = SKILL_CATALOG_PAGE;

// --- rendering -------------------------------------------------------------

function renderSkillUpdateBadge() {
  const badge = $('skillUpdateBadge');
  const n = skillView ? skillView.updates : 0;
  if (n > 0) {
    badge.textContent = n + ' 个更新';
    badge.classList.remove('hidden');
  } else {
    badge.classList.add('hidden');
  }
}

function renderSkillWarning() {
  const warn = $('skillWarning');
  if (skillView && skillView.warning) {
    warn.textContent = skillView.warning + '（重启应用会自动修复；也可尝试重新安装对应技能包）';
    warn.classList.remove('hidden');
  } else {
    warn.classList.add('hidden');
  }
}

function originLabel(origin) {
  return origin === 'local' ? '本地' : origin === 'git' ? 'git' : 'npm';
}

// Whether the workbench is currently serving: decides whether an action
// toast says「即时生效」or「下次启动可用」. currentView is a top-level let
// binding in app.js — same global lexical scope, referenced bare.
function kernelRunningNow() {
  return Boolean(typeof currentView !== 'undefined' && currentView && currentView.kernel && currentView.kernel.running);
}

function effectSuffix() {
  return kernelRunningNow() ? '，对运行中的工作台即时生效' : '，下次启动工作台后可见';
}

function renderSkillList() {
  const list = $('skillList');
  list.innerHTML = '';
  const hint = $('skillRootHint');
  if (skillView && skillView.skills_root) {
    hint.textContent = '内核读取目录：' + skillView.skills_root;
    hint.classList.remove('hidden');
  }
  if (!skillView || !skillView.rows || skillView.rows.length === 0) {
    list.appendChild(el('p', 'muted', '尚未安装任何技能包。'));
    return;
  }
  skillView.rows.forEach((row) => {
    const item = el('div', 'installed-row plugin-row');

    const info = el('span', 'plugin-info');
    info.appendChild(el('span', 'release-ver', row.name));
    const pinNote = row.pinned && row.origin !== 'local' ? ' · 已锁定版本' : '';
    const upgrade = row.latest_version ? ' → ' + row.latest_version : '';
    info.appendChild(el(
      'span',
      'plugin-meta',
      originLabel(row.origin) + ' · ' + (row.installed_version || '—') + upgrade + pinNote
       + ' · ' + row.skills.length + ' 个技能'
    ));
    if (row.description) {
      info.appendChild(el('span', 'plugin-meta', row.description));
    }
    item.appendChild(info);

    // One chip per skill: frontmatter name + short description + toggle.
    const chips = el('span', 'skill-chips');
    row.skills.forEach((entry) => {
      const chip = el('span', 'skill-chip' + (entry.enabled ? ' enabled' : ''));
      chip.appendChild(el('span', 'skill-chip-name', entry.name));
      if (entry.description) {
        const desc = entry.description.length > 60
          ? entry.description.slice(0, 57) + '…'
          : entry.description;
        chip.appendChild(el('span', 'skill-chip-desc', desc));
      }
      if (entry.enabled && !entry.present) {
        chip.appendChild(el('span', 'badge warn', '待修复'));
      }
      chip.appendChild(mkBtn(entry.enabled ? '停用' : '启用', () =>
        toggleSkill(row.id, entry.name, !entry.enabled), 'ghost'));
      chips.appendChild(chip);
    });
    item.appendChild(chips);

    const actions = el('span', 'release-actions plugin-actions');
    if (row.actual_mode === 'copy') {
      actions.appendChild(el('span', 'badge', '复制'));
    } else if (row.actual_mode === 'link') {
      actions.appendChild(el('span', 'badge installed', '链接'));
    }
    if (row.latest_version) {
      actions.appendChild(el('span', 'badge update', '有更新 ' + row.latest_version));
      actions.appendChild(mkBtn('更新', () => updateSkill(row.id)));
    } else if (row.origin === 'local') {
      // Local packages have no version feed; 更新 doubles as re-sync from
      // the source folder after edits.
      actions.appendChild(mkBtn('重新同步', () => updateSkill(row.id), 'ghost'));
    }
    if (row.repo_url) {
      actions.appendChild(mkBtn('仓库', () => openExternal(row.repo_url), 'ghost'));
    }
    actions.appendChild(mkBtn('卸载', (ev) => armConfirm(ev.currentTarget, {
      armedLabel: '确认卸载？',
      idleLabel: '卸载',
      onConfirm: (btn) => proceedSkillRemove(row.id, btn),
    }), 'danger'));
    item.appendChild(actions);

    list.appendChild(item);
  });
}

// --- skill center ----------------------------------------------------------

function skillInstalledKeys() {
  const keys = new Set();
  const rows = (skillView && skillView.rows) || [];
  rows.forEach((row) => {
    keys.add(String(row.name || '').toLowerCase());
    if (row.repo_url) {
      keys.add(row.repo_url.replace(/^https?:\/\/github\.com\//i, '').replace(/\.git$/, '').toLowerCase());
    }
  });
  return keys;
}

function skillIsInstalled(item, keys) {
  if (keys.has(String(item.name || '').toLowerCase())) return true;
  return item.repo ? keys.has(item.repo.toLowerCase()) : false;
}

function filteredSkillCatalog(keys) {
  const q = $('skillCatalogQuery').value.trim().toLowerCase();
  const filter = $('skillCatalogFilter').value;
  return skillCatalogItems.filter((item) => {
    if (filter === 'installed' && !skillIsInstalled(item, keys)) return false;
    if (filter === 'not-installed' && skillIsInstalled(item, keys)) return false;
    if (!q) return true;
    const hay = [item.name, item.description, item.repo, item.category].join(' ').toLowerCase();
    return hay.includes(q);
  });
}

function renderSkillCatalogCard(item, keys) {
  const card = el('div', 'catalog-card');
  const head = el('div', 'catalog-card-head');
  const title = el('span', 'catalog-title');
  title.appendChild(el('span', 'catalog-name', item.name));
  if (item.version) title.appendChild(el('span', 'badge', item.version));
  if (item.category) title.appendChild(el('span', 'badge cat', item.category));
  if (item.verified) title.appendChild(el('span', 'badge installed', '已验证'));
  head.appendChild(title);
  head.appendChild(el('span', 'catalog-stats', item.stars > 0 ? '★ ' + item.stars : ''));
  card.appendChild(head);

  if (item.description) {
    card.appendChild(el('p', 'catalog-desc',
      item.description.length > 140 ? item.description.slice(0, 137) + '…' : item.description));
  }

  const foot = el('div', 'catalog-card-foot');
  foot.appendChild(el('span', 'catalog-tags'));
  const actions = el('span', 'catalog-actions');
  const detailUrl = item.detail_url || (item.repo ? 'https://github.com/' + item.repo : '');
  if (detailUrl) {
    actions.appendChild(mkBtn('打开详情', () => openExternal(detailUrl), 'ghost'));
  }
  if (skillIsInstalled(item, keys)) {
    const btn = el('button', 'ghost', '已安装');
    btn.type = 'button';
    btn.disabled = true;
    actions.appendChild(btn);
  } else if (item.spec) {
    actions.appendChild(mkBtn('安装', () => installSkill(item.spec)));
  }
  foot.appendChild(actions);
  card.appendChild(foot);
  return card;
}

function renderSkillCatalog() {
  const list = $('skillCatalogList');
  list.innerHTML = '';
  if (!skillCatalogLoaded) {
    list.appendChild(el('p', 'muted', '目录加载中…'));
    return;
  }
  const keys = skillInstalledKeys();
  const items = filteredSkillCatalog(keys);
  $('skillCatalogCount').textContent = skillCatalogItems.length
    ? '结果 ' + items.length + ' 条 · 收录 ' + skillCatalogItems.length + ' 款'
    : '';
  if (items.length === 0) {
    list.appendChild(el('p', 'muted', skillCatalogItems.length
      ? '没有匹配的技能，换个关键词试试。'
      : '社区技能目录暂未提供数据。可在 GitHub dsh-skill topic 浏览，或把仓库地址 / 本地文件夹路径粘贴到上方手动安装。'));
  } else {
    items.slice(0, skillCatalogShown).forEach((item) => list.appendChild(renderSkillCatalogCard(item, keys)));
  }
  $('btnSkillCatalogMore').classList.toggle('hidden', items.length <= skillCatalogShown);
}

function loadSkillCatalog(manual) {
  skillCatalogLoaded = false;
  renderSkillCatalog();
  // Same narrow guard as the plugin center: only the refresh button locks;
  // everything else stays interactive during the network read.
  const reload = $('btnSkillCatalogReload');
  const more = $('btnSkillCatalogMore');
  if (manual) {
    reload.disabled = true;
    if (more) more.disabled = true;
  }
  return invoke('skill_catalog', { force: !!manual })
    .then((items) => {
      skillCatalogItems = items || [];
      skillCatalogLoaded = true;
      skillCatalogShown = SKILL_CATALOG_PAGE;
      renderSkillCatalog();
    })
    .catch((e) => {
      skillCatalogLoaded = true;
      renderSkillCatalog();
      if (manual) {
        toast('目录加载失败：' + e, 6000);
      }
    })
    .finally(() => {
      if (manual) {
        reload.disabled = false;
        if (more) more.disabled = false;
      }
    });
}

// --- actions ---------------------------------------------------------------
//
// Long tasks run through withPluginProgress (plugins.js) so the progress
// panel, log stream, busy lock, and failure UX stay identical across the
// two cards. Its done/fail labels carry skill-specific wording; the
// effect suffix explains the live-update behavior to the user.

function installSkill(spec) {
  const input = $('skillSpec');
  const fromCatalogOrArg = !!(spec || '').trim();
  const raw = (spec || '').trim() || input.value.trim();
  if (!raw) {
    toast('请先填写 npm 包名、仓库地址或本地文件夹路径', 4000);
    return;
  }
  if (!fromCatalogOrArg) {
    input.value = '';
  }
  const mode = $('skillMode').value;
  return withPluginProgress(
    {
      cmd: 'skill_install',
      start: '正在安装技能包 ' + raw + ' …',
      done: '技能包 ' + raw + ' 已安装' + effectSuffix(),
      fail: '安装失败：' + raw
    },
    (channel) => ({ spec: raw, mode, onEvent: channel })
  );
}

function updateSkill(id) {
  return withPluginProgress(
    {
      cmd: 'skill_update',
      start: '正在更新技能包 …',
      done: '技能包已更新' + effectSuffix()
    },
    (channel) => ({ id, onEvent: channel })
  );
}

function toggleSkill(id, name, enabled) {
  return withPluginProgress(
    {
      cmd: 'skill_set_enabled',
      start: '正在' + (enabled ? '启用' : '停用') + '技能 ' + name + ' …',
      done: '技能 ' + name + ' 已' + (enabled ? '启用' : '停用') + effectSuffix()
    },
    (channel) => ({ id, name, enabled, onEvent: channel })
  );
}

function proceedSkillRemove(id, btn) {
  btn.disabled = true;
  return withPluginProgress(
    {
      cmd: 'skill_uninstall',
      start: '正在卸载技能包 …',
      done: '技能包已卸载',
      fail: '卸载失败'
    },
    (channel) => ({ id, onEvent: channel })
  );
}

// Manual checks pass { busy: true }; the startup self-check stays silent.
function checkSkillUpdates(opts) {
  if (opts.busy) {
    setBusy(true);
  }
  return invoke('skill_check_updates')
    .then((infos) => {
      const n = (infos || []).filter((i) => i.latest).length;
      if (n > 0 && opts.toastOnUpdates) {
        toast('有 ' + n + ' 个技能包可更新', 5000);
      }
      return refreshAll();
    })
    .catch((e) => {
      if (opts.busy) {
        toast('检查技能更新失败：' + e, 6000);
      }
    })
    .finally(() => {
      if (opts.busy) {
        setBusy(false);
      }
    });
}

// --- wiring ----------------------------------------------------------------

$('btnSkillInstall').addEventListener('click', () => installSkill(''));
$('btnSkillCheck').addEventListener('click', () => checkSkillUpdates({ busy: true, toastOnUpdates: true }));
$('btnSkillCatalogReload').addEventListener('click', () => loadSkillCatalog(true));
$('btnSkillCatalogMore').addEventListener('click', () => {
  skillCatalogShown += SKILL_CATALOG_PAGE;
  renderSkillCatalog();
});
let skillCatalogQueryTimer = null;
$('skillCatalogQuery').addEventListener('input', () => {
  clearTimeout(skillCatalogQueryTimer);
  skillCatalogQueryTimer = setTimeout(() => {
    skillCatalogShown = SKILL_CATALOG_PAGE;
    renderSkillCatalog();
  }, 150);
});
$('skillCatalogFilter').addEventListener('change', () => {
  skillCatalogShown = SKILL_CATALOG_PAGE;
  renderSkillCatalog();
});
$('skillSpec').addEventListener('keydown', (ev) => {
  if (ev.key === 'Enter') {
    installSkill('');
  }
});

// app.js 的 refreshAll 会调用这个钩子，与内核状态、插件状态一起刷新技能卡片。
window.__dshSkillsRefresh = () => {
  return invoke('skill_status')
    .then((view) => {
      skillView = view;
      renderSkillUpdateBadge();
      renderSkillWarning();
      renderSkillList();
      if (skillCatalogLoaded) {
        renderSkillCatalog();
      }
    })
    .catch(() => {
      // 静默刷新：技能状态读取失败时保留旧卡片，下次 refreshAll 再试。
    });
};

// 启动后预载社区目录（静默，失败不打断），随后静默检查一次更新。
setTimeout(() => {
  loadSkillCatalog(false);
}, 1600);
setTimeout(() => {
  checkSkillUpdates({ busy: false, toastOnUpdates: true });
}, 4200);
