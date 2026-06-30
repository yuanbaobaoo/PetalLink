<!-- 侧边栏，递归目录树 + 账号栏（含配额） -->
<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { MateAppLogo } from "@/components/mate";
import SidebarTreeNode from "./SidebarTreeNode.vue";
import { useFileBrowserStore, ROOT } from "@/stores/fileBrowser";
import { useAuthStore } from "@/stores/auth";
import { useUpdaterStore } from "@/stores/updater";
import * as authApi from "@/api/auth";
import * as driveApi from "@/api/drive";
import type { DriveAbout } from "@/api/drive";

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

/**
 * 挂载后获取存储配额信息
 */
onMounted(async () => {
  try { about.value = await driveApi.getAbout(); } catch { about.value = null; }
});

function fmtSize(bytes: number): string {
  if (!bytes) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), u.length - 1);
  return `${(bytes / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 1)} ${u[i]}`;
}
</script>

<template>
  <aside class="sidebar">
    <!-- Logo 区 -->
    <div class="sidebar__logo">
      <MateAppLogo :size="26" />
    </div>

    <!-- 目录树（递归） -->
    <div class="sidebar__tree">
      <SidebarTreeNode :location="ROOT" :path="[ROOT]" :depth="0" :active-id="browser.current.id" />
    </div>

    <!-- 底部账号栏 -->
    <div class="sidebar__account">
      <div class="account-avatar">{{ userInitial }}</div>
      <div class="account-info">
        <div class="account-info__primary">{{ userLabel }}</div>
        <div v-if="quotaText" class="account-info__secondary">{{ quotaText }}</div>
      </div>
    </div>

    <!-- 更新包下载进度条：正在下载更新包时显示，点击重新打开弹窗 -->
    <div
      v-if="updater.isUpdateDownloading"
      class="sidebar__update-progress"
      @click="updater.showDownloadDialog()"
      title="点击查看更新详情"
    >
      <div class="update-progress__head">
        <span class="update-progress__label">正在下载更新</span>
        <span class="update-progress__pct">{{ updater.downloadProgress }}%</span>
      </div>
      <div class="update-progress__bar">
        <div class="update-progress__fill" :style="{ width: `${updater.downloadProgress}%` }" />
      </div>
    </div>

    <!-- 更新提示条：有可用更新时显示 -->
    <div
      v-if="updater.updateAvailable && !updater.dismissed"
      class="sidebar__update-banner"
      @click="updater.showDialog()"
    >
      <svg class="update-banner__icon" viewBox="0 0 16 16" width="14" height="14" fill="none">
        <path d="M8 1.5a5.5 5.5 0 0 0-1 10.91v1.34a.5.5 0 0 0 .8.4L9.75 12.5h.17A5.5 5.5 0 0 0 8 1.5Z" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round"/>
        <circle cx="8" cy="8" r="1.25" fill="currentColor"/>
      </svg>
      <span class="update-banner__text">新版本 {{ updater.newVersion }}</span>
      <button class="update-banner__close" @click.stop="updater.dismissUpdate" title="关闭">×</button>
    </div>
  </aside>
</template>

<style scoped>
.sidebar { width: var(--sidebar-width); display: flex; flex-direction: column; background-color: var(--bg-page); border-right: 0.5px solid var(--border); flex-shrink: 0; }
.sidebar__logo { display: flex; align-items: center; height: var(--appbar-height); padding: 0 var(--space-lg); flex-shrink: 0; }
.sidebar__tree { flex: 1; overflow-y: auto; padding: var(--space-xs) var(--space-sm); }
.sidebar__account { display: flex; align-items: center; gap: var(--space-md); padding: var(--space-lg); border-top: 0.5px solid var(--border); flex-shrink: 0; }
.account-avatar { width: 28px; height: 28px; border-radius: 50%; background: linear-gradient(135deg, var(--color-brand), var(--color-brand-hover)); color: #fff; font-size: var(--font-body-sm); font-weight: var(--fw-semibold); display: flex; align-items: center; justify-content: center; flex-shrink: 0; }
.account-info { flex: 1; min-width: 0; }
.account-info__primary { font-size: var(--font-body-sm); font-weight: var(--fw-medium); color: var(--text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.account-info__secondary { font-size: var(--font-caption); color: var(--text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

/* 更新包下载进度条（账号栏下方） */
.sidebar__update-progress {
  padding: var(--space-sm) var(--space-lg) var(--space-md);
  border-top: 0.5px solid var(--border);
  cursor: pointer;
  flex-shrink: 0;
  transition: background-color 0.15s;
}
.sidebar__update-progress:hover { background-color: var(--bg-hover); }
.update-progress__head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
.update-progress__label { font-size: var(--font-caption); color: var(--text-secondary); }
.update-progress__pct { font-size: var(--font-caption); font-weight: var(--fw-semibold); color: var(--color-brand); }
.update-progress__bar { height: 4px; background-color: var(--bg-active); border-radius: 2px; overflow: hidden; }
.update-progress__fill { height: 100%; background: linear-gradient(90deg, var(--color-brand), var(--color-brand-hover)); border-radius: 2px; transition: width 0.3s ease; }

/* 更新提示条 */
.sidebar__update-banner {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: var(--space-sm) var(--space-lg);
  margin: 0 var(--space-sm) var(--space-sm);
  background: linear-gradient(135deg, var(--color-brand), var(--color-brand-hover));
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: opacity 0.15s;
  flex-shrink: 0;
}
.sidebar__update-banner:hover { opacity: 0.9; }
.update-banner__icon { color: #fff; flex-shrink: 0; }
.update-banner__text {
  flex: 1;
  font-size: var(--font-caption);
  font-weight: var(--fw-medium);
  color: #fff;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.update-banner__close {
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  border: none;
  border-radius: 50%;
  background: rgba(255,255,255,0.25);
  color: #fff;
  font-size: 14px;
  line-height: 1;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}
.update-banner__close:hover { background: rgba(255,255,255,0.4); }
</style>
