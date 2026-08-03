<!-- 首次同步引导条 -->
<script setup lang="ts">
import { ref } from "vue";
import { useSyncStore } from "@/stores/sync";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { MateInfoBanner, MateButton } from "@/components/mate";
import { commands } from "@/api/generated";
import * as configApi from "@/api/config";
import { open } from "@tauri-apps/plugin-dialog";
import { useAsyncAction } from "@/composables/useAsyncAction";
import { extractErrorMessage } from "@/utils/error";
import { selectAndConfigureSyncDirectory } from "@/composables/useSyncDirectorySetup";
import { isCompletelyEmptyDir } from "@/utils/fs";
import { isLinuxPlatform } from "@/utils/platform";

// 当前同步状态。
const sync = useSyncStore();
// 当前文件浏览器状态。
const browser = useFileBrowserStore();
// 当前操作的错误提示。
const errorMessage = ref("");
// 目录选择按钮的互斥执行状态。
const { loading: selectDirLoading, run: runSelectDir } = useAsyncAction();
// Linux 只提供 FUSE 云盘目录；其他平台继续使用传统同步目录。
const isLinux = isLinuxPlatform();

/**
 * 选择用户可见目录，并按平台提交传统同步目录或 FUSE 云盘目录。
 */
async function handleSelectDir(): Promise<void> {
  await runSelectDir(async () => {
    try {
      // 首次配置基于当前完整持久化配置提交。
      const config = await configApi.loadConfig();

      if (!isLinux) {
        // 非 Linux 继续走 1.1.4 的统一目录配置与刷新入口。
        const result = await selectAndConfigureSyncDirectory(config);
        if (!result) return;
      } else {
        // Linux 用户只选择 FUSE 可见挂载目录，不能覆盖隐藏 backing。
        const selected = await open({
          directory: true,
          multiple: false,
          title: "选择云盘目录",
        });
        if (!selected || typeof selected !== "string") return;

        if (!(await isCompletelyEmptyDir(selected))) {
          errorMessage.value = "所选目录不为空。请选择一个完全空的目录作为云盘目录。";
          return;
        }

        await commands.configSave(
          configApi.withSelectedDriveDirectory(config, selected, true),
        );
        // 保存成功后立即提交配置事实，再做配置与文件列表收敛刷新。
        sync.applyMountConfiguration(selected);
        await sync.init();
        await browser.loadRoot();
      }
      errorMessage.value = "";
    } catch (e) {
      errorMessage.value = `${isLinux ? "配置云盘目录" : "配置同步目录"}失败：`
        + extractErrorMessage(e);
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

  <!-- Linux 云盘配置已完成，但真实 FUSE 会话尚未可用。 -->
  <div
    v-else-if="isLinux && sync.mountConfigured && !sync.virtualDriveMounted"
    class="setup-banner setup-banner--warning"
  >
    <MateInfoBanner variant="warning" class="setup-banner__inner">
      {{ sync.virtualDriveError
        ? `云盘挂载失败：${sync.virtualDriveError}`
        : "云盘正在启动，完成后即可在文件管理器中使用。" }}
      <template #action>
        <MateButton variant="text" icon="refresh" @click="sync.refreshVirtualDriveStatus()">
          刷新状态
        </MateButton>
      </template>
    </MateInfoBanner>
  </div>

  <!-- needsSetup：尚未配置用户可见目录 -->
  <div v-else-if="sync.setupPhase === 'needsSetup'" class="setup-banner setup-banner--info">
    <MateInfoBanner variant="info" class="setup-banner__inner">
      {{ isLinux
        ? "尚未配置云盘目录，选择一个空目录后即可在文件管理器中使用"
        : "尚未配置同步目录，选择一个空目录开始同步" }}
      <template #action>
        <MateButton
          variant="text"
          icon="folder-open"
          :loading="selectDirLoading"
          :disabled="selectDirLoading"
          @click="handleSelectDir"
        >
          {{ isLinux ? "选择云盘目录" : "选择目录" }}
        </MateButton>
      </template>
    </MateInfoBanner>
  </div>

  <!-- needsFirstSync：目录已就绪，等待首次同步 -->
  <div v-else-if="sync.setupPhase === 'needsFirstSync'" class="setup-banner setup-banner--warning">
    <MateInfoBanner variant="warning" class="setup-banner__inner">
      {{ isLinux ? "云盘目录" : "同步目录" }}已就绪：{{ sync.userVisibleRoot || '未配置' }}，点击「同步索引」读取云端文件
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
