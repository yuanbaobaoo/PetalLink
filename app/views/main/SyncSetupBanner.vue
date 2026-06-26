<!-- 首次同步引导条 -->
<script setup lang="ts">
import { ref } from "vue";
import { useSyncStore } from "@/stores/sync";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { MateInfoBanner, MateButton } from "@/components/mate";
import * as configApi from "@/api/config";
import { open } from "@tauri-apps/plugin-dialog";
import { readDir } from "@tauri-apps/plugin-fs";

const sync = useSyncStore();
const browser = useFileBrowserStore();
const errorMessage = ref("");

/**
 * 检查目录是否为空（过滤隐藏文件 + skipPatterns）
 */
async function isEmptyDir(dir: string): Promise<boolean> {
  const entries = await readDir(dir);
  const skipPatterns = [".DS_Store", ".tmp", "~$*", ".Trash"];
  const visible = entries.filter((e) => {
    const name = e.name ?? "";
    if (!name) return false;
    if (name.startsWith(".")) return false; // 隐藏文件
    for (const p of skipPatterns) {
      if (p.includes("*")) {
        if (new RegExp("^" + p.replace(/\./g, "\\.").replace(/\*/g, ".*")).test(name)) return false;
      } else if (name === p) return false;
    }
    return true;
  });
  return visible.length === 0;
}

/**
 * 选择同步目录：原生目录选择器 → 校验空目录 → 保存配置 → 重新评估阶段
 */
async function handleSelectDir(): Promise<void> {
  const selected = await open({ directory: true, multiple: false, title: "选择同步目录" });
  if (!selected || typeof selected !== "string") return;

  // 校验：必须空目录（过滤隐藏文件 + skipPatterns 后）
  try {
    if (!(await isEmptyDir(selected))) {
      errorMessage.value = "所选目录不为空。请选择一个空目录作为同步目录，避免与已有文件冲突。";
      return;
    }
  } catch (e) {
    errorMessage.value = "检查目录失败：" + ((e as { message?: string }).message ?? String(e));
    return;
  }

  try {
    const config = await configApi.loadConfig();
    config.mount_dir = selected;
    config.mount_configured = true;
    await configApi.saveConfig(config);
    await sync.init();
    // 刷新文件列表（引擎在 saveConfig 内已启动，正在 BFS + 创建本地占位符）
    await browser.loadRoot();
    errorMessage.value = "";
  } catch (e) {
    errorMessage.value = "配置同步目录失败：" + ((e as { message?: string }).message ?? String(e));
  }
}

async function handleFirstSync(): Promise<void> {
  try {
    await sync.triggerManualRefresh();
    errorMessage.value = "";
  } catch (e) {
    errorMessage.value = "首次同步失败：" + ((e as { message?: string }).message ?? String(e));
  }
}

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
        <MateButton variant="text" icon="folder-open" @click="handleSelectDir">选择目录</MateButton>
      </template>
    </MateInfoBanner>
  </div>

  <!-- needsFirstSync：目录已就绪，等待首次同步 -->
  <div v-else-if="sync.setupPhase === 'needsFirstSync'" class="setup-banner setup-banner--warning">
    <MateInfoBanner variant="warning" class="setup-banner__inner">
      同步目录已就绪：{{ sync.mountDir || '未配置' }}，点击「同步索引」拉取云端索引
      <template #action>
        <MateButton variant="text" icon="sync" @click="handleFirstSync">同步索引</MateButton>
      </template>
    </MateInfoBanner>
  </div>
</template>

<style scoped>
.setup-banner {
  padding: var(--space-xs) var(--space-lg);
  border-bottom: 0.5px solid var(--border);
  background-color: var(--bg-container);
  flex-shrink: 0;
}
.setup-banner__inner { width: 100%; }
</style>
