<script setup>
// 日志面板：顶部贴合的弹层，左侧竖排日志文件签（含大小），右侧按需读取内容。
import { nextTick, ref, watch } from 'vue';
import { Refresh, Close } from '@element-plus/icons-vue';
import { logModal, formatLogSize, switchLogTab, loadActiveLog, hideLogs } from '../logs.js';

const tabsBox = ref(null);

// 切签 / 列表刷新后把激活签滚进可视区。
watch(
  () => logModal.activeName,
  async () => {
    await nextTick();
    const active = tabsBox.value && tabsBox.value.querySelector('.log-tab[aria-selected="true"]');
    if (active) {
      active.scrollIntoView({ block: 'nearest' });
    }
  }
);
</script>

<template>
  <el-dialog
    v-model="logModal.visible"
    title="日志"
    top="12px"
    width="min(860px, 92vw)"
    class="log-dialog"
    :show-close="false"
    append-to-body
  >
    <template #header>
      <div style="display: flex; align-items: center; gap: 8px">
        <span style="font-weight: 700; font-size: 15px">日志</span>
        <span style="flex: 1"></span>
        <el-button text :icon="Refresh" :loading="logModal.loading" title="重新读取当前日志" @click="loadActiveLog">
          刷新
        </el-button>
        <el-button text :icon="Close" @click="hideLogs">关闭</el-button>
      </div>
    </template>
    <div class="log-main">
      <div ref="tabsBox" class="log-tabs" role="tablist" aria-orientation="vertical">
        <span v-if="!logModal.files.length" class="log-tab-size">（暂无日志文件）</span>
        <button
          v-for="f in logModal.files"
          :key="f.name"
          type="button"
          class="log-tab"
          role="tab"
          :aria-selected="f.name === logModal.activeName ? 'true' : 'false'"
          :title="f.name"
          @click="switchLogTab(f.name)"
        >
          <span>{{ f.name }}</span>
          <span v-if="typeof f.size === 'number'" class="log-tab-size">{{ formatLogSize(f.size) }}</span>
        </button>
      </div>
      <pre class="log-content">{{ logModal.content }}</pre>
    </div>
  </el-dialog>
</template>
