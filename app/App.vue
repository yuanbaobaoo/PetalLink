<!--
  PetalLink 应用根组件 —— 根据 auth 状态路由：
  - initial + loading：启动闪屏
  - loggedIn：主界面 或 设置页
  - loggedOut / error / authorizing：登录页
-->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAuthStore } from "@/stores/auth";
import { useSyncStore } from "@/stores/sync";
import { useUpdaterStore, CHECK_INTERVAL_MS } from "@/stores/updater";
import { on } from "@/api/tauri";
import LoginPage from "@/views/LoginPage.vue";
import MainPage from "@/views/main/MainPage.vue";
import SettingsPage from "@/views/settings/SettingsPage.vue";
import LogViewerPage from "@/views/settings/LogViewerPage.vue";
import IconSprite from "@/components/IconSprite.vue";
import UpdateDialog from "@/components/UpdateDialog.vue";
import { MateAppLogo, MateDialogHost, MateToastHost } from "@/components/mate";

const auth = useAuthStore();
// 当前页面：main / settings / logs
const currentPage = ref<"main" | "settings" | "logs">("main");

const showSplash = computed(() => auth.status === "initial" && auth.loading);
const showMain = computed(() => auth.status === "loggedIn");

// 定时器 / 事件监听句柄（onUnmounted 时清理）
let initialCheckTimer: ReturnType<typeof setTimeout> | null = null;
let periodicCheckTimer: ReturnType<typeof setInterval> | null = null;
let unlistenFocus: UnlistenFn | null = null;

/**
 * 启动时恢复登录态 + 初始化同步 + 注册全局事件 + 更新检查
 */
  onMounted(async () => {
  await auth.restore();
  if (auth.status === "loggedIn") {
    const sync = useSyncStore();
    await sync.init();
  }
  // 注册全局事件：打开设置页
  try {
    await on("navigate_settings", () => openSettings());
  } catch {}

  // 启动后延迟静默检查更新（不阻塞启动流程）
  const updater = useUpdaterStore();
  // ① 首次检查（启动 3s 后，强制不节流）
  initialCheckTimer = setTimeout(() => {
    updater.silentCheck();
  }, 3000);

  // ② 每 1 小时定时检查（内部 1 小时节流，重复触发也不会超频）
  periodicCheckTimer = setInterval(() => {
    updater.periodicCheck();
  }, CHECK_INTERVAL_MS);

  // ③ 窗口获得焦点时检查（节流 10 分钟）——覆盖从后台恢复、托盘/Dock 点击、
  //   单实例聚焦等所有「主窗口重新显示」的路径
  getCurrentWindow()
    .onFocusChanged(({ payload: focused }) => {
      if (focused) updater.checkOnFocus();
    })
    .then((fn) => {
      unlistenFocus = fn;
    });
});

onUnmounted(() => {
  if (initialCheckTimer) clearTimeout(initialCheckTimer);
  if (periodicCheckTimer) clearInterval(periodicCheckTimer);
  unlistenFocus?.();
});

/** 显示设置页（全局事件，MainPage 通过 emit 触发） */
function openSettings(): void { currentPage.value = "settings"; }
/** 返回主界面 */
function openMain(): void { currentPage.value = "main"; }
/** 显示日志页（设置页触发） */
function openLogs(): void { currentPage.value = "logs"; }
</script>

<template>
    <!-- 全局 SVG 图标 sprite（display:none，仅供 <MateIcon> <use> 引用） -->
    <IconSprite />
    <!-- 全局对话框 / Toast 宿主（模块级状态，任意处 await confirmDialog / showToast） -->
    <MateDialogHost />
    <MateToastHost />
    <!-- 更新对话框（独立于全局 dialog 系统，有自己的状态机） -->
    <UpdateDialog />

  <div v-if="showSplash" class="splash">
    <MateAppLogo :size="56" />
    <p class="splash__status">正在初始化…</p>
  </div>

  <SettingsPage v-else-if="showMain && currentPage === 'settings'" @back="openMain" @open-logs="openLogs" />
  <LogViewerPage v-else-if="showMain && currentPage === 'logs'" @back="openSettings" />
  <MainPage v-else-if="showMain" @open-settings="openSettings" />
  <LoginPage v-else />
</template>

<style scoped>
.splash {
  width: 100%; height: 100%; display: flex; flex-direction: column;
  align-items: center; justify-content: center; gap: var(--space-xl);
  background-color: var(--bg-page);
}
.splash__status { font-size: var(--font-body-sm); color: var(--text-secondary); }
</style>
