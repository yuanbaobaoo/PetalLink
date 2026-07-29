<!-- 首次同步引导条 -->
<script setup lang="ts">
import { ref } from "vue";
import { useSyncStore } from "@/stores/sync";
import { MateInfoBanner, MateButton } from "@/components/mate";
import * as configApi from "@/api/config";
import { useAsyncAction } from "@/composables/useAsyncAction";
import { extractErrorMessage } from "@/utils/error";
import { selectAndConfigureSyncDirectory } from "@/composables/useSyncDirectorySetup";

// 当前同步状态。
const sync = useSyncStore();
// 当前操作的错误提示。
const errorMessage = ref("");
// 目录选择按钮的互斥执行状态。
const { loading: selectDirLoading, run: runSelectDir } = useAsyncAction();

/**
 * 通过统一入口选择并首次配置同步目录。
 */
async function handleSelectDir(): Promise<void> {
  await runSelectDir(async () => {
    try {
      // 首次配置基于当前完整持久化配置提交。
      const config = await configApi.loadConfig();
      // 统一目录配置结果。
      const result = await selectAndConfigureSyncDirectory(config);
      if (!result) return;
      errorMessage.value = "";
    } catch (e) {
      errorMessage.value = "配置同步目录失败：" + extractErrorMessage(e);
    }
  });
}

/**
 * 执行首次同步并刷新配置阶段。
 */
async function handleFirstSync(): Promise<void> {
  try {
    await sync.triggerManualRefresh();
    errorMessage.value = "";
  } catch (e) {
    errorMessage.value = "首次同步失败：" + extractErrorMessage(e);
  }
}

/**
 * 重新执行上一次失败的初始化动作。
 */
async function handleRetry(): Promise<void> {
  errorMessage.value = "";
  await sync.init();
}
</script>

<template>
  <!-- error 态 -->
  <div v-if="errorMessage" class="setup-banner setup-banner--error">
    <MateInfoBanner variant="error" class="setup-banner__inner">
      {{ errorMessage }}
      <template #action>
        <MateButton variant="text" icon="refresh" @click="handleRetry">重试</MateButton>
      </template>
    </MateInfoBanner>
  </div>

  <!-- needsSetup：尚未配置同步目录 -->
  <div v-else-if="sync.setupPhase === 'needsSetup'" class="setup-banner setup-banner--info">
    <MateInfoBanner variant="info" class="setup-banner__inner">
      尚未配置同步目录，选择一个空目录开始同步
      <template #action>
        <MateButton
          variant="text"
          icon="folder-open"
          :loading="selectDirLoading"
          :disabled="selectDirLoading"
          @click="handleSelectDir"
        >
          选择目录
        </MateButton>
      </template>
    </MateInfoBanner>
  </div>

  <!-- needsFirstSync：目录已就绪，等待首次同步 -->
  <div v-else-if="sync.setupPhase === 'needsFirstSync'" class="setup-banner setup-banner--warning">
    <MateInfoBanner variant="warning" class="setup-banner__inner">
      同步目录已就绪：{{ sync.mountDir || '未配置' }}，点击「同步索引」读取云端文件
      <template #action>
        <MateButton variant="text" icon="sync" @click="handleFirstSync">同步索引</MateButton>
      </template>
    </MateInfoBanner>
  </div>
</template>

<style scoped>
.setup-banner {
  padding: var(--space-sm) 20px;
  border-bottom: 1px solid var(--line);
  background-color: var(--bg-card);
  flex-shrink: 0;
}
.setup-banner__inner { width: 100%; }
</style>
