<!--
  更新对话框：确认更新 → 下载进度 → 等待传输 → 重启
  由 App.vue 根据 updater store 的 phase 控制显隐
-->
<script setup lang="ts">
import { computed } from "vue";
import { MateButton, MateIcon } from "@/components/mate";
import { useUpdaterStore } from "@/stores/updater";
import { relaunch } from "@tauri-apps/plugin-process";

const updater = useUpdaterStore();

const visible = computed(() => {
  return updater.dialogOpen && (
    updater.phase === "available" ||
    updater.phase === "downloading" ||
    updater.phase === "downloaded" ||
    updater.phase === "waitingTransfer" ||
    updater.phase === "ready" ||
    updater.phase === "error"
  );
});

const title = computed(() => {
  switch (updater.phase) {
    case "available": return "发现新版本";
    case "downloading": return "正在下载更新…";
    case "downloaded":
    case "waitingTransfer": return "下载完成";
    case "ready": return "更新就绪";
    case "error": return "更新失败";
    default: return "";
  }
});

const versionLabel = computed(() => {
  return updater.updateInfo?.version
    ? `v${updater.updateInfo.version}`
    : "";
});

const releaseNotes = computed(() => {
  return updater.updateInfo?.body ?? "";
});

const transferWaiting = computed(() => updater.phase === "waitingTransfer");

async function handleStartUpdate(): Promise<void> {
  await updater.downloadAndInstall();
  // 下载完成后自动等待传输
  if (updater.phase === "downloaded") {
    const ok = await updater.waitForTransfers();
    if (ok && updater.phase === "ready") {
      await relaunch();
    }
  }
}

async function handleRetry(): Promise<void> {
  await updater.downloadAndInstall();
}

async function handleRelaunch(): Promise<void> {
  await relaunch();
}

function handleClose(): void {
  updater.closeDialog();
}
</script>

<template>
  <Teleport to="body">
    <div v-if="visible" class="update-overlay" @click.self="handleClose">
      <div class="update-dialog">
        <!-- 头部 -->
        <div class="update-dialog__header">
          <MateIcon name="download" :size="20" class="update-dialog__icon" />
          <span class="update-dialog__title">{{ title }}</span>
        </div>

        <!-- 版本号 -->
        <div v-if="versionLabel" class="update-dialog__version">{{ versionLabel }}</div>

        <!-- 正文区 -->
        <div class="update-dialog__body">
          <!-- 确认态 -->
          <template v-if="updater.phase === 'available'">
            <div v-if="releaseNotes" class="update-dialog__notes">
              <div class="update-dialog__notes-label">更新内容</div>
              <pre class="update-dialog__notes-text">{{ releaseNotes }}</pre>
            </div>
            <p v-else class="update-dialog__hint">是否下载并安装此更新？</p>
          </template>

          <!-- 下载中 -->
          <template v-else-if="updater.phase === 'downloading'">
            <div class="update-dialog__progress">
              <div class="update-dialog__progress-bar">
                <div
                  class="update-dialog__progress-fill"
                  :style="{ width: `${updater.downloadProgress}%` }"
                />
              </div>
              <span class="update-dialog__progress-text">{{ updater.downloadProgress }}%</span>
            </div>
          </template>

          <!-- 等待传输 -->
          <template v-else-if="updater.phase === 'downloaded' || transferWaiting">
            <div class="update-dialog__waiting">
              <div class="update-dialog__spinner" />
              <p>下载完成。{{ transferWaiting ? '等待所有文档上传/下载完成后自动重启…' : '准备安装…' }}</p>
            </div>
          </template>

          <!-- 就绪 -->
          <template v-else-if="updater.phase === 'ready'">
            <p class="update-dialog__hint">更新已准备就绪，重启即可生效。</p>
          </template>

          <!-- 错误 -->
          <template v-else-if="updater.phase === 'error'">
            <p class="update-dialog__error">{{ updater.errorMessage || "更新失败，请稍后重试。" }}</p>
          </template>
        </div>

        <!-- 底部按钮 -->
        <div class="update-dialog__footer">
          <template v-if="updater.phase === 'available'">
            <MateButton variant="text" @click="handleClose">稍后提醒</MateButton>
            <MateButton variant="primary" icon="download" @click="handleStartUpdate">立即更新</MateButton>
          </template>
          <template v-else-if="updater.phase === 'error'">
            <MateButton variant="text" @click="handleClose">关闭</MateButton>
            <MateButton variant="primary" icon="refresh" @click="handleRetry">重试</MateButton>
          </template>
          <template v-else-if="updater.phase === 'ready'">
            <MateButton variant="text" @click="handleClose">稍后</MateButton>
            <MateButton variant="primary" icon="check" @click="handleRelaunch">立即重启</MateButton>
          </template>
          <template v-else-if="updater.phase === 'downloaded' || transferWaiting">
            <MateButton variant="text" @click="handleClose">后台等待</MateButton>
            <MateButton variant="primary" icon="check" @click="handleRelaunch" :disabled="transferWaiting">
              {{ transferWaiting ? '等待传输完成…' : '立即重启' }}
            </MateButton>
          </template>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.update-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.3);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1001;
}
.update-dialog {
  width: 440px;
  max-width: calc(100vw - 48px);
  background-color: var(--bg-container);
  border: 0.5px solid rgba(0, 0, 0, 0.2);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-modal);
  display: flex;
  flex-direction: column;
  animation: dialog-fade-in 0.15s ease-out;
}
.update-dialog__header {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: var(--space-xxl) var(--space-xxl) var(--space-xs) var(--space-xxl);
}
.update-dialog__icon {
  color: var(--color-brand);
  flex-shrink: 0;
}
.update-dialog__title {
  font-size: var(--font-title-sm);
  font-weight: var(--fw-semibold);
  color: var(--text-primary);
}
.update-dialog__version {
  padding: 0 var(--space-xxl);
  font-size: var(--font-body-lg);
  font-weight: var(--fw-bold);
  color: var(--color-brand);
}
.update-dialog__body {
  padding: var(--space-md) var(--space-xxl) var(--space-xxl);
  font-size: var(--font-body);
  line-height: 1.6;
  color: var(--text-secondary);
}
.update-dialog__hint {
  margin: 0;
  color: var(--text-secondary);
}
.update-dialog__notes {
  margin-bottom: var(--space-md);
}
.update-dialog__notes-label {
  font-size: var(--font-caption);
  font-weight: var(--fw-semibold);
  color: var(--text-secondary);
  margin-bottom: var(--space-xs);
}
.update-dialog__notes-text {
  margin: 0;
  padding: var(--space-md);
  background: var(--bg-page);
  border-radius: var(--radius-md);
  font-size: var(--font-caption);
  line-height: 1.5;
  color: var(--text-secondary);
  max-height: 180px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: inherit;
}
.update-dialog__progress {
  display: flex;
  align-items: center;
  gap: var(--space-md);
}
.update-dialog__progress-bar {
  flex: 1;
  height: 8px;
  background: var(--bg-page);
  border-radius: 4px;
  overflow: hidden;
}
.update-dialog__progress-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--color-brand), var(--color-brand-hover));
  border-radius: 4px;
  transition: width 0.3s ease;
}
.update-dialog__progress-text {
  font-size: var(--font-caption);
  font-weight: var(--fw-semibold);
  color: var(--text-primary);
  min-width: 40px;
  text-align: right;
}
.update-dialog__waiting {
  display: flex;
  align-items: flex-start;
  gap: var(--space-md);
}
.update-dialog__spinner {
  width: 20px;
  height: 20px;
  border: 2.5px solid var(--border);
  border-top-color: var(--color-brand);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
  margin-top: 2px;
}
@keyframes spin {
  to { transform: rotate(360deg); }
}
.update-dialog__error {
  color: var(--color-error);
  margin: 0;
}
.update-dialog__footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
  padding: var(--space-sm) var(--space-lg) var(--space-lg);
  border-top: 0.5px solid var(--border);
}
</style>
