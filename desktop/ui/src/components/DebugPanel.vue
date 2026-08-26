<script setup>
// dev 调试浮按钮 + 弹层。
//
// 仅在 dev 构建（body.dev-build）下渲染，提供「在 dev 里临时启用 release 版 CSS 钩子」的预览入口：
//   - 勾选「预览 release 顶部绿渐变」会把 rel-build 类挂到 body 上，复用 theme.css:150 那条
//     body.rel-build::before 规则，让 dev 期即可看到 64px 绿色渐变带。
//   - 真正的状态变更走 store.setReleasePreview()，非 dev 构建会被拒绝（不污染正式版）。
//   - 弹层内同时列出 release-only 的 CSS 钩子与代码位置，方便对照调试。
//
// 设计原则：不依赖 Tauri 桥、不发起 IO、不写本地文件；纯 UI 状态切换。
import { ref, computed } from 'vue';
import { store, setReleasePreview } from '../store.js';

const open = ref(false);

// 兜底：组件首次渲染时 refreshAll 可能还没把 store.view 写进来，
// 直接读 body 上的 dev-build 类避免 UI 闪烁或漏显（HMR 时已设过类）。
const fallbackHasDevClass = ref(
  typeof document !== 'undefined' && document.body.classList.contains('dev-build')
);

// 仅 dev 构建显示：正式版 store.view.dev_build===false，直接不渲染。
const isDev = computed(() => !!store.view?.dev_build || fallbackHasDevClass.value);

// release 预览开关：通过 store 动作改，poll 同步类不会冲掉用户的覆盖。
const previewRel = computed({
  get: () => !!store.releasePreview,
  set: (v) => setReleasePreview(v),
});

function toggle() {
  open.value = !open.value;
}

function close() {
  open.value = false;
}
</script>

<template>
  <div v-if="isDev" class="debug-fab-wrap">
    <button
      class="debug-fab"
      :class="{ open }"
      type="button"
      :aria-expanded="open"
      aria-label="dev 调试面板"
      title="dev 调试面板 · 仅 dev 构建可见"
      @click="toggle"
    >
      <span aria-hidden="true">{{ open ? '×' : '🪛' }}</span>
    </button>

    <section
      v-if="open"
      class="debug-panel"
      role="dialog"
      aria-label="dev 调试面板"
      @click.stop
    >
      <header>
        <h3>dev 调试</h3>
        <span class="debug-hint">仅 dev 构建可见</span>
        <button
          class="debug-close"
          type="button"
          aria-label="关闭"
          title="关闭"
          @click="close"
        >
          ×
        </button>
      </header>

      <h4>release 版 CSS 钩子预览</h4>
      <ul class="debug-list">
        <li class="debug-row">
          <label>
            <input v-model="previewRel" type="checkbox" />
            <span>预览 release 顶部绿渐变</span>
          </label>
          <small class="debug-meta">
            <code>body.rel-build::before</code>
            · linear-gradient rgba(96, 153, 38, 0.20 → 0)
            · Gitea brand 绿 <code>--gitea-green #609926</code>
            · height 64px
            · 幅度与 dev 构建鲸眼红 (20%) 对齐
          </small>
        </li>
      </ul>

      <h4>release-only 钩子清单</h4>
      <ul class="debug-hooks">
        <li>
          <code>theme.css:150</code>
          <span>body.rel-build::before — 顶 64px 绿渐变</span>
        </li>
        <li>
          <code>store.js:applyBuildClass</code>
          <span>dev_build=false → rel-build 类切换</span>
        </li>
        <li>
          <code>OverviewPanel.vue</code>
          <span>桌面端版本号去掉「（dev）」后缀</span>
        </li>
        <li>
          <code>desktop/src-tauri</code>
          <span>build.rs 中的 dev_build 标志（决定 Rust 调用 get_status 时返回什么）</span>
        </li>
      </ul>

      <footer class="debug-foot">
        <span>本面板仅本地调试用，不会读写文件、不触发 IO。</span>
      </footer>
    </section>
  </div>
</template>
