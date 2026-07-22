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

// 头部 icon badge 的配色类名（v2：品牌 / 成功 / 失败）
const badgeClass = computed(() => {
  if (updater.phase === "ready") return "update-dialog__badge--ok";
  if (updater.phase === "error") return "update-dialog__badge--err";
  return "";
});

async function handleStartUpdate(): Promise<void> {
  await updater.downloadAndInstall();
  // 下载完成后自动等待传输
  if (updater.phase === "downloaded") {
    const ok = await updater.waitForTransfers();
    if (ok) {
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
        <!-- 头部（v2：icon badge + 标题） -->
        <div class="update-dialog__header">
          <span class="update-dialog__badge" :class="badgeClass">
            <MateIcon name="download" :size="20" />
          </span>
          <div class="update-dialog__headtext">
            <span class="update-dialog__title">{{ title }}</span>
            <span v-if="versionLabel" class="update-dialog__version">{{ versionLabel }}</span>
          </div>
        </div>

        <!-- 正文区 -->
        <div class="update-dialog__body">
          <!-- 确认态 -->
          <template v-if="updater.phase === 'available'">
            <div v-if="releaseNotes" class="update-dialog__notes">
              <div class="update-dialog__notes-label">更新内容</div>
              <pre class="update-dialog__notes-text">{{ releaseNotes }}</pre>
            </div>
            <p v-else class="update-dialog__hint">暂无更新日志。是否下载并安装此更新？</p>
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
          <template v-else-if="updater.phase === 'downloading'">
            <MateButton variant="text" @click="handleClose">后台下载</MateButton>
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
  background: rgba(28, 28, 30, 0.36);
  backdrop-filter: blur(3px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1001;
}
.update-dialog {
  width: 460px;
  max-width: calc(100vw - 48px);
  background-color: var(--bg-card);
  border-radius: var(--radius-xl);
  box-shadow: var(--sh-pop);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: dialog-fade-in 0.15s ease-out;
}
.update-dialog__header {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: var(--space-xl) var(--space-xl) var(--space-sm);
}
/* v2 icon badge（40px 圆角 10px） */
.update-dialog__badge {
  width: 40px;
  height: 40px;
  border-radius: 10px;
  flex-shrink: 0;
  background: var(--brand-50);
  color: var(--brand-500);
  display: flex;
  align-items: center;
  justify-content: center;
}
.update-dialog__badge--ok { background: var(--ok-bg); color: var(--ok); }
.update-dialog__badge--err { background: var(--err-bg); color: var(--err); }
.update-dialog__headtext {
  display: flex;
  align-items: baseline;
  gap: var(--space-sm);
  min-width: 0;
}
.update-dialog__title {
  font-size: 17px;
  font-weight: var(--fw-semibold);
  color: var(--ink-900);
}
.update-dialog__version {
  font-size: var(--font-body-sm);
  font-weight: var(--fw-semibold);
  color: var(--brand-500);
  font-variant-numeric: tabular-nums;
}
.update-dialog__body {
  padding: var(--space-sm) var(--space-xl) 20px;
  font-size: var(--font-body);
  line-height: 1.65;
  color: var(--ink-600);
}
.update-dialog__hint {
  margin: 0;
  color: var(--ink-600);
}
.update-dialog__notes {
  margin-bottom: var(--space-md);
}
.update-dialog__notes-label {
  font-size: var(--font-caption);
  font-weight: var(--fw-semibold);
  color: var(--ink-400);
  margin-bottom: var(--space-xs);
}
/* v2 changelog：灰底面板 */
.update-dialog__notes-text {
  margin: 0;
  padding: var(--space-md);
  background: var(--bg-fill);
  border-radius: var(--radius-md);
  font-size: var(--font-caption);
  line-height: 1.6;
  color: var(--ink-600);
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
  height: 6px;
  background: var(--bg-fill);
  border-radius: var(--radius-full);
  overflow: hidden;
}
.update-dialog__progress-fill {
  height: 100%;
  background: var(--grad-brand);
  border-radius: var(--radius-full);
  transition: width 0.3s ease;
}
.update-dialog__progress-text {
  font-size: var(--font-caption);
  font-weight: var(--fw-semibold);
  color: var(--ink-900);
  min-width: 40px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.update-dialog__waiting {
  display: flex;
  align-items: flex-start;
  gap: var(--space-md);
}
.update-dialog__spinner {
  width: 18px;
  height: 18px;
  border: 2.5px solid var(--brand-100);
  border-top-color: var(--brand-500);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
  margin-top: 2px;
}
@keyframes spin {
  to { transform: rotate(360deg); }
}
.update-dialog__error {
  color: var(--err);
  margin: 0;
}
.update-dialog__footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 0 var(--space-xl) 20px;
}
</style>
