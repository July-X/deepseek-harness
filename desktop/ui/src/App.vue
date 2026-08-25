<script setup>
// 应用骨架：侧栏 + 面板切换 + 全局浮层（进度 / 日志 / 事故），
// 以及启动时的事件监听、轮询与静默自检的编排。
import { onMounted, onUnmounted } from 'vue';
import { invoke, listen } from './bridge.js';
import { toastError, confirmDialog } from './notify.js';
import {
  store,
  refreshAll,
  pollStatus,
  checkShellUpdate,
  showShellUpdateBanner,
} from './store.js';
import { loadCatalog, checkPluginUpdates } from './plugins.js';
import { checkSkillUpdates } from './skills.js';
import SideBar from './components/SideBar.vue';
import OverviewPanel from './components/OverviewPanel.vue';
import VersionsPanel from './components/VersionsPanel.vue';
import PluginsPanel from './components/PluginsPanel.vue';
import SkillsPanel from './components/SkillsPanel.vue';
import SettingsPanel from './components/SettingsPanel.vue';
import ProgressOverlay from './components/ProgressOverlay.vue';
import LogModal from './components/LogModal.vue';
import IncidentModal from './components/IncidentModal.vue';

const PANELS = {
  overview: OverviewPanel,
  versions: VersionsPanel,
  plugins: PluginsPanel,
  skills: SkillsPanel,
  settings: SettingsPanel,
};

let pollTimer = null;
const startupTimers = [];

// 完全退出确认：Rust 侧在内核仍在运行时拦截主窗口关闭（prevent_close），
// 由这里弹确认框；用户确认后先停内核（释放端口）再销毁窗口。
// pending 标记压住用户在弹窗期间连续点 X 的重入。
let quitConfirmPending = false;
function onQuitConfirmRequest() {
  if (quitConfirmPending) return;
  quitConfirmPending = true;
  confirmDialog(
    '完全退出？',
    '工作台仍在运行。关闭主壳前需要先关闭工作台；继续吗？',
    '关闭并退出'
  )
    .then((ok) => {
      if (!ok) return null;
      return invoke('stop_kernel')
        .catch((e) => toastError('关闭工作台失败：' + e))
        .then(() => invoke('confirm_close_shell'))
        .catch((e) => toastError('退出失败：' + e + '（请手动关闭窗口）', 6000));
    })
    .finally(() => {
      quitConfirmPending = false;
    });
}

function onVisibilityChange() {
  if (!document.hidden) {
    pollStatus();
  }
}

onMounted(() => {
  refreshAll();

  // 状态轮询：窗口隐藏时整个跳过；重新可见时立即补一轮。
  pollTimer = setInterval(pollStatus, 2500);
  document.addEventListener('visibilitychange', onVisibilityChange);

  // 外壳后台检查到新版后广播此事件；手动按钮覆盖按需检查。
  listen('shell-update-available', (e) => showShellUpdateBanner(e.payload));
  listen('request-quit-confirm', onQuitConfirmRequest);

  // 启动后的静默预载 / 自检：失败均不打断，用户可在对应面板手动重试。
  startupTimers.push(setTimeout(() => loadCatalog(false), 1200));
  startupTimers.push(setTimeout(() => checkPluginUpdates({ busy: false, toastOnUpdates: true }), 3500));
  startupTimers.push(setTimeout(() => checkSkillUpdates({ busy: false, toastOnUpdates: true }), 4200));
  startupTimers.push(setTimeout(() => checkShellUpdate(false), 2500));
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
  startupTimers.forEach(clearTimeout);
  document.removeEventListener('visibilitychange', onVisibilityChange);
});
</script>

<template>
  <div class="layout">
    <SideBar />
    <main>
      <Transition name="panel" mode="out-in">
        <component :is="PANELS[store.activePanel]" :key="store.activePanel" />
      </Transition>
    </main>
  </div>
  <ProgressOverlay />
  <LogModal />
  <IncidentModal />
</template>
