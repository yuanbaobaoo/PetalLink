<!-- 侧边栏（v2：毛玻璃 248px），logo 区 + 目录树 + 更新卡片 + 账号卡 -->
<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { MateAppLogo, MateIcon } from "@/components/mate";
import SidebarTreeNode from "./SidebarTreeNode.vue";
import { useFileBrowserStore, ROOT } from "@/stores/fileBrowser";
import { useAuthStore } from "@/stores/auth";
import { useUpdaterStore } from "@/stores/updater";
import * as authApi from "@/api/auth";
import * as driveApi from "@/api/drive";
import type { DriveAbout } from "@/api/drive";
import { formatFileSize } from "@/utils/format";

// 账号加载中的占位文本
const LOADING_LABEL = "加载账号中…";
// 头像占位首字符
const FALLBACK_INITIAL = "华";

const browser = useFileBrowserStore();
const auth = useAuthStore();
const updater = useUpdaterStore();

const userLabel = computed(() => authApi.primaryLabel(auth.userInfo) ?? LOADING_LABEL);
const userInitial = computed(() => authApi.initial(auth.userInfo) ?? FALLBACK_INITIAL);

// 配额
const about = ref<DriveAbout | null>(null);
const quotaText = computed(() => {
  if (!about.value || about.value.user_capacity <= 0) return "";
  return `${fmtSize(about.value.used_space)} / ${fmtSize(about.value.user_capacity)}`;
});
// 配额使用百分比（0-100），用于账号卡 4px 进度条
const quotaPercent = computed(() => {
  if (!about.value || about.value.user_capacity <= 0) return 0;
  return Math.min(100, Math.round((about.value.used_space / about.value.user_capacity) * 100));
});

/**
 * 挂载后获取存储配额信息
 */
onMounted(async () => {
  try { about.value = await driveApi.getAbout(); } catch { about.value = null; }
});

function fmtSize(bytes: number): string {
  // 配额场景：0 字节显示 "0 B" 而非 "—"（与原行为一致，表示已用 0 字节）
  if (!bytes) return "0 B";
  return formatFileSize(bytes);
}
</script>

<template>
  <aside class="sidebar">
    <!-- Logo 区 -->
    <div class="sidebar__logo">
      <MateAppLogo :size="28" />
    </div>

    <!-- 位置分组 -->
    <div class="sidebar__section">位置</div>

    <!-- 目录树（递归） -->
    <div class="sidebar__tree">
      <SidebarTreeNode :location="ROOT" :path="[ROOT]" :depth="0" :active-id="browser.current.id" />
    </div>

    <!-- 更新卡片（v2：渐变品牌卡）：下载中显示进度，否则有可用更新时显示横幅 -->
    <div
      v-if="updater.isUpdateDownloading"
      class="sidebar__update"
      title="点击查看更新详情"
      @click="updater.showDownloadDialog()"
    >
      <div class="sidebar__update-title">
        <span>正在下载更新</span>
        <span class="sidebar__update-pct">{{ updater.downloadProgress }}%</span>
      </div>
      <div class="sidebar__update-track">
        <div class="sidebar__update-fill" :style="{ width: `${updater.downloadProgress}%` }" />
      </div>
    </div>
    <div v-else-if="updater.updateAvailable && !updater.dismissed" class="sidebar__update">
      <div class="sidebar__update-title">
        <span>新版本 {{ updater.newVersion }}</span>
        <button class="sidebar__update-close" title="关闭" @click.stop="updater.dismissUpdate">
          <MateIcon name="x" :size="12" />
        </button>
      </div>
      <button class="sidebar__update-btn" @click="updater.showDialog()">
        <MateIcon name="download" :size="14" />
        立即更新
      </button>
    </div>

    <!-- 底部账号卡 -->
    <div class="sidebar__account">
      <div class="account-avatar">{{ userInitial }}</div>
      <div class="account-info">
        <div class="account-info__primary">{{ userLabel }}</div>
        <div v-if="quotaText" class="account-info__secondary">{{ quotaText }}</div>
        <div v-if="quotaText" class="account-info__quota-track">
          <div class="account-info__quota-fill" :style="{ width: `${quotaPercent}%` }" />
        </div>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: var(--sidebar-width);
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: rgba(247, 247, 249, 0.85);
  backdrop-filter: blur(20px);
  border-right: 1px solid var(--line);
  flex-shrink: 0;
}
.sidebar__logo {
  display: flex;
  align-items: center;
  height: 60px;
  padding: 0 18px;
  flex-shrink: 0;
}
.sidebar__section {
  font-size: 11px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.4px;
  color: var(--ink-300);
  padding: 12px 18px 6px;
  flex-shrink: 0;
}
.sidebar__tree {
  flex: 1;
  overflow-y: auto;
  padding: 0 10px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

/* 底部账号卡（v2：白卡 + 配额进度条） */
.sidebar__account {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 10px;
  padding: var(--space-md);
  background: var(--bg-card);
  border-radius: var(--radius-lg);
  box-shadow: var(--sh-sm), 0 0 0 0.5px var(--line);
  flex-shrink: 0;
}
.account-avatar {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  background: var(--grad-brand);
  color: #fff;
  font-size: var(--font-body);
  font-weight: var(--fw-semibold);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.account-info { flex: 1; min-width: 0; }
.account-info__primary {
  font-size: var(--font-body-sm);
  font-weight: var(--fw-medium);
  color: var(--ink-900);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.account-info__secondary {
  font-size: var(--font-caption);
  color: var(--ink-400);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.account-info__quota-track {
  height: 4px;
  margin-top: 6px;
  background: var(--bg-fill);
  border-radius: var(--radius-full);
  overflow: hidden;
}
.account-info__quota-fill {
  height: 100%;
  background: var(--grad-brand);
  border-radius: var(--radius-full);
}

/* 更新卡片（v2：渐变品牌卡，侧栏底部） */
.sidebar__update {
  margin: 0 10px 10px;
  border-radius: var(--radius-lg);
  background: var(--grad-brand);
  color: #fff;
  padding: var(--space-md);
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
  box-shadow: var(--sh-brand);
  cursor: pointer;
  flex-shrink: 0;
}
.sidebar__update-title {
  font-size: var(--font-body-sm);
  font-weight: var(--fw-semibold);
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.sidebar__update-pct {
  font-size: var(--font-caption);
  font-variant-numeric: tabular-nums;
}
.sidebar__update-close {
  width: 20px;
  height: 20px;
  border: none;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.25);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  flex-shrink: 0;
}
.sidebar__update-close:hover { background: rgba(255, 255, 255, 0.4); }
.sidebar__update-btn {
  height: 28px;
  border: none;
  border-radius: var(--radius-sm);
  background: rgba(255, 255, 255, 0.95);
  color: var(--brand-500);
  font-size: var(--font-caption);
  font-weight: var(--fw-semibold);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-xs);
  cursor: pointer;
}
.sidebar__update-track {
  height: 4px;
  background: rgba(255, 255, 255, 0.3);
  border-radius: var(--radius-full);
  overflow: hidden;
}
.sidebar__update-fill {
  height: 100%;
  background: #fff;
  border-radius: var(--radius-full);
  transition: width 0.3s ease;
}
</style>
