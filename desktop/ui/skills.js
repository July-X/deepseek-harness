'use strict';

// Skill management card: renders the central store (one row per package),
// drives install/update/uninstall, and surfaces update reminders.
// Loaded after app.js and plugins.js; reuses their helpers ($, invoke,
// toast, el, mkBtn, openExternal) plus withPluginProgress from plugins.js
// so every long task shares the same progress panel and log stream.
//
// v1 only ships the manual-install row (same shape as the plugin
// panel's manual install) plus update checks; a community catalog card
// is deliberately out of scope. There is no per-skill enable/disable UI:
// 卸载即停用，不单独暴露「启停」概念。

let skillView = null;

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
// binding in app.js - same global lexical scope, referenced bare.
function kernelRunningNow() {
  return Boolean(typeof currentView !== 'undefined' && currentView && currentView.kernel && currentView.kernel.running);
}

function effectSuffix() {
  return kernelRunningNow() ? '，对运行中的工作台即时生效' : '，下次启动工作台后可见';
}

function renderSkillList() {
  const list = $('skillList');
  list.innerHTML = '';
  if (!skillView || !skillView.rows || skillView.rows.length === 0) {
    list.appendChild(el('p', 'muted', '尚未安装任何技能包。'));
    return;
  }
  skillView.rows.forEach((row) => {
    const item = el('div', 'installed-row plugin-row');

    // 第一行：包名 + 来源/版本（左）与 tag / 操作按钮（右）同行显示；
    // 第二行才是每技能一枚的启停 chip。
    const head = el('span', 'skill-row-head');
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
    head.appendChild(info);

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
    head.appendChild(actions);
    item.appendChild(head);

    list.appendChild(item);
  });
}

// --- actions ---------------------------------------------------------------
//
// Long tasks run through withPluginProgress (plugins.js) so the progress
// panel, log stream, busy lock, and failure UX are identical across the
// two cards. The done / fail labels carry skill-specific wording; the
// effect suffix explains the live-update behavior to the user.

function installSkill() {
  const input = $('skillSpec');
  const raw = input.value.trim();
  if (!raw) {
    toast('请先填写 git 仓库地址，例如 https://github.com/owner/repo.git', 4000);
    return;
  }
  input.value = '';
  return withPluginProgress(
    {
      cmd: 'skill_install',
      start: '正在安装技能包 ' + raw + ' …',
      done: '技能包 ' + raw + ' 已安装' + effectSuffix() + '；新开一个工作台会话即可调用。',
      fail: '安装失败：' + raw
    },
    (channel) => ({ spec: raw, onEvent: channel })
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

$('btnSkillCheck').addEventListener('click', () => checkSkillUpdates({ busy: true, toastOnUpdates: true }));
$('skillSpec').addEventListener('keydown', (ev) => {
  if (ev.key === 'Enter') {
    installSkill();
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
    })
    .catch(() => {
      // 静默刷新：技能状态读取失败时保留旧卡片，下次 refreshAll 再试。
    });
};

// 启动后静默检查一次更新，有新版时提醒。
setTimeout(() => {
  checkSkillUpdates({ busy: false, toastOnUpdates: true });
}, 4200);