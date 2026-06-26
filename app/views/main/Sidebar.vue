<!-- 侧边栏，递归目录树 + 账号栏（含配额） -->
<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { MateAppLogo } from "@/components/mate";
import SidebarTreeNode from "./SidebarTreeNode.vue";
import { useFileBrowserStore, ROOT } from "@/stores/fileBrowser";
import { useAuthStore } from "@/stores/auth";
import * as authApi from "@/api/auth";
import * as driveApi from "@/api/drive";
import type { DriveAbout } from "@/api/drive";

// 账号加载中的占位文本
const LOADING_LABEL = "加载账号中…";
// 头像占位首字符
const FALLBACK_INITIAL = "华";

const browser = useFileBrowserStore();
const auth = useAuthStore();

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
</style>
