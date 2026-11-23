<!--
  登录页 —— v2 原型 01-login：品牌渐变画布 + 居中玻璃拟态卡片。

  布局：
  - 柔光渐变背景（径向品牌蓝光斑 + 浅色线性渐变）
  - 3 个低透明度装饰圆
  - 居中玻璃卡片（480px，白 90% + blur 16，radius-xl + sh-pop）
  - 卡片内：品牌图标 + 标题 + 渐变分隔线 + 可选 warning/error banner + 主按钮
  - 授权中：替换为 spinner 条 + 取消按钮
-->
<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { events } from "@/api/generated";
import { useAuthStore } from "@/stores/auth";
import {
  MateButton,
  MateAppLogo,
  MateIcon,
  MateInfoBanner,
  MateCircularProgress,
  showToast,
} from "@/components/mate";
import { useAppIdentity } from "@/composables/useAppIdentity";
import { commands } from "@/api/generated";
import { extractErrorMessage } from "@/utils/error";
import {
  computeSheetRect,
  sheetContentRect,
  SHEET_HEADER_HEIGHT,
  SHEET_RADIUS,
} from "@/utils/sheet";

// 当前认证状态。
const auth = useAuthStore();

// 应用完整标题（dev 包自带 -Dev 后缀）。
const { fullTitle: appTitle } = useAppIdentity();

// client_secret 未配置时的提示文案
const secretWarning =
  "尚未配置 client_secret。请在项目根目录创建 .env 文件，写入：\nHWCLOUD_CLIENT_SECRET=<你的 64 位 hex>\n（参考 .env.example；或在启动命令中 --dart-define=HWCLOUD_CLIENT_SECRET=...）";

// 登录按钮是否可点（secret 已配置且非加载中）
const canLogin = computed(
  () => auth.secretConfigured && !auth.loading
);

// 是否展示授权进行中状态。
const showAuthorizing = computed(
  () => auth.loading && auth.status === "authorizing"
);

// 标题区右上角按钮文案（内嵌被拦截时改用系统浏览器）。
const OPEN_IN_BROWSER_LABEL = "改用系统浏览器";
// 标题区右上角关闭按钮文案。
const CLOSE_LABEL = "关闭";
// 底部抽屉是否已展开（原生登录 webview 由后端嵌进内容区）。
const sheetOpen = ref(false);
// 窗口内容区尺寸，随 resize 更新（驱动抽屉几何与原生视图重新定位）。
const viewport = ref({ width: window.innerWidth, height: window.innerHeight });
// 抽屉外框（含标题栏）。
const sheetRect = computed(() => computeSheetRect(viewport.value));
// 抽屉内容区（原生 webview 的实际位置）。
// sheetHeight 决定滑入行程；viewportHeight 让后端算出标题栏偏移，原生子视图才能对齐 CSS。
const sheetContent = computed(() => ({
  ...sheetContentRect(sheetRect.value),
  sheetHeight: Math.round(sheetRect.value.height),
  viewportHeight: Math.round(viewport.value.height),
}));
// 抽屉外框的内联样式。
const sheetStyle = computed(() => ({
  left: `${sheetRect.value.x}px`,
  top: `${sheetRect.value.y}px`,
  width: `${sheetRect.value.width}px`,
  height: `${sheetRect.value.height}px`,
  // 只圆上面两角：浮层贴底，下角保持直角。
  borderRadius: `${SHEET_RADIUS}px ${SHEET_RADIUS}px 0 0`,
}));

/**
 * 记录最新窗口尺寸；抽屉打开时同步原生视图位置。
 */
function handleViewportResize(): void {
  viewport.value = { width: window.innerWidth, height: window.innerHeight };
  if (!sheetOpen.value) return;
  // 原生视图不会跟着 CSS 走，必须显式通知后端重新定位。
  void commands.authLoginSheetBounds(sheetContent.value).catch(() => {});
}

// 后端上报登录界面真实形态时取消订阅。
let unlistenLoginLaunched: (() => void) | null = null;

onMounted(async () => {
  window.addEventListener("resize", handleViewportResize);
  unlistenLoginLaunched = await events.authLoginLaunched.listen(({ payload }) => {
    // 只有抽屉形态才需要保留外壳；退回独立窗口/浏览器时立刻收起，避免留下空白块。
    if (payload.mode !== "sheet") {
      sheetOpen.value = false;
    }
  });
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", handleViewportResize);
  unlistenLoginLaunched?.();
});

/**
 * 开始 OAuth 登录流程：先展开底部抽屉，等 Vue 完成布局后再把内容区矩形交给后端。
 */
async function handleLogin(): Promise<void> {
  sheetOpen.value = true;
  // 等抽屉 DOM 落地：几何必须与后端嵌入的 webview 完全一致。
  await nextTick();
  const started = await auth.login(sheetContent.value);
  if (!started) {
    sheetOpen.value = false;
  }
}

/**
 * 用系统浏览器继续当前授权流程。
 *
 * 内嵌登录窗口被华为拦截（或页面无法加载）时的回退入口：
 * loopback 回调服务仍在等待，浏览器里完成的授权同样会回到本应用。
 */
async function handleOpenInBrowser(): Promise<void> {
  try {
    await commands.authLoginInBrowser();
    showToast("已用系统浏览器打开登录页");
  } catch (e) {
    showToast("打开浏览器失败：" + extractErrorMessage(e), { variant: "error" });
  }
}

/**
 * 取消当前 OAuth 授权流程。
 */
function handleCancel(): void {
  sheetOpen.value = false;
  auth.cancelLogin();
}

/**
 * 清除登录错误并返回可重试状态。
 */
function handleDismissError(): void {
  auth.dismissError();
}
</script>

<template>
  <!-- 登录浮层遮罩：点击空白处等效取消 -->
  <div
    v-if="sheetOpen"
    class="login-scrim"
    @click="handleCancel"
  />

  <!-- 登录浮层外壳：圆角与标题区由 Vue 绘制，原生 webview 填在内容区 -->
  <section
    v-if="sheetOpen"
    class="login-sheet"
    :style="sheetStyle"
    role="dialog"
    aria-modal="true"
    aria-label="登录华为账号"
  >
    <header class="login-sheet__header" :style="{ height: `${SHEET_HEADER_HEIGHT}px` }">
      <div class="login-sheet__heading">
        <h2 class="login-sheet__title">登录华为账号</h2>
      </div>
      <div class="login-sheet__actions">
        <!-- 内嵌被身份提供方拦截时，用系统浏览器继续同一次授权流程 -->
        <button
          class="login-sheet__icon-btn"
          type="button"
          :title="OPEN_IN_BROWSER_LABEL"
          :aria-label="OPEN_IN_BROWSER_LABEL"
          @click="handleOpenInBrowser"
        >
          <MateIcon name="external" :size="16" />
        </button>
        <button
          class="login-sheet__icon-btn"
          type="button"
          :title="CLOSE_LABEL"
          :aria-label="CLOSE_LABEL"
          @click="handleCancel"
        >
          <MateIcon name="x" :size="16" />
        </button>
      </div>
    </header>
    <!-- 原生登录页会覆盖在这块区域上，这里只作占位 -->
    <div class="login-sheet__content" />
  </section>

  <div class="login-page" :class="{ 'login-page--dimmed': sheetOpen }">
    <!-- 装饰圆（低透明度品牌色） -->
    <div class="decor-circle decor-circle--lg" />
    <div class="decor-circle decor-circle--md" />
    <div class="decor-circle decor-circle--sm" />

    <!-- 居中玻璃卡片 -->
    <div class="login-card">
      <!-- 品牌容器图标 -->
      <MateAppLogo container text="" />

      <!-- 标题 -->
      <h1 class="login-card__title">{{ appTitle }}</h1>

      <!-- 渐变分隔线 -->
      <div class="login-card__divider" />

      <!-- secret 未配置警告 -->
      <MateInfoBanner
        v-if="!auth.secretConfigured"
        variant="warning"
        class="login-card__banner"
      >
        {{ secretWarning }}
      </MateInfoBanner>

      <!-- 错误 banner（带重新授权按钮） -->
      <MateInfoBanner
        v-if="auth.errorMessage"
        variant="error"
        class="login-card__banner"
      >
        {{ auth.errorMessage }}
        <template #action>
          <MateButton variant="text" @click="handleDismissError">重新授权</MateButton>
        </template>
      </MateInfoBanner>

      <!-- 主按钮区 -->
      <div class="login-card__actions">
        <!-- 授权中面板 -->
        <div v-if="showAuthorizing" class="authorizing-pane">
          <div class="authorizing-pane__bar">
            <MateCircularProgress :size="16" :stroke-width="2" />
            <span class="authorizing-pane__text">请在登录浮层中完成授权...</span>
          </div>
          <!-- 内嵌窗口被身份提供方拦截时，用系统浏览器继续同一次流程 -->
          <MateButton variant="text" icon="link" class="authorizing-pane__cancel" @click="handleOpenInBrowser">
            改用系统浏览器
          </MateButton>
          <MateButton variant="text" icon="x" class="authorizing-pane__cancel" @click="handleCancel">
            取消授权
          </MateButton>
        </div>

        <!-- 登录按钮 -->
        <MateButton
          v-else
          variant="primary"
          icon="cloud"
          full-width
          :height="46"
          class="login-card__login-btn"
          :disabled="!canLogin"
          @click="handleLogin"
        >
          使用华为账号登录
        </MateButton>
      </div>

      <!-- 底部说明 -->
      <p class="login-card__hint">在应用内完成授权，支持账号密码或手机扫码登录</p>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  position: relative;
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  /* v2：径向品牌蓝光斑 + 浅色线性渐变 */
  background:
    radial-gradient(720px 420px at 85% -10%, #dce8fc 0%, transparent 60%),
    radial-gradient(640px 420px at 0% 110%, #eff4fe 0%, transparent 55%),
    linear-gradient(150deg, #fbfbfc 0%, #f5f5f7 100%);
}

/* 装饰圆（品牌色低透明度） */
.decor-circle {
  position: absolute;
  border-radius: 50%;
  pointer-events: none;
  background-color: var(--brand-500);
}
.decor-circle--lg {
  width: 400px;
  height: 400px;
  top: -100px;
  right: -80px;
  opacity: 0.06;
}
.decor-circle--md {
  width: 300px;
  height: 300px;
  bottom: -60px;
  left: -80px;
  opacity: 0.06;
}
.decor-circle--sm {
  width: 200px;
  height: 200px;
  top: 45%;
  left: 30%;
  opacity: 0.05;
}

/* 登录卡片（v2：玻璃拟态） */
.login-card {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
  max-width: 480px;
  padding: 44px 40px 32px;
  background: rgba(255, 255, 255, 0.9);
  backdrop-filter: blur(16px);
  border-radius: var(--radius-xl);
  box-shadow: var(--sh-pop), 0 0 0 0.5px rgba(0, 0, 0, 0.04);
}

.login-card__title {
  margin-top: 14px;
  font-size: var(--font-title-md);
  font-weight: var(--fw-semibold);
  color: var(--ink-900);
  letter-spacing: -0.2px;
  text-align: center;
}

.login-card__divider {
  width: 44px;
  height: 3px;
  margin-top: 10px;
  background: var(--grad-brand);
  border-radius: var(--radius-full);
}

.login-card__banner {
  width: 100%;
  margin-top: var(--space-lg);
}

.login-card__actions {
  width: 100%;
  margin-top: 36px;
}

.login-card__login-btn {
  border-radius: var(--radius-lg);
  font-size: 15px;
}

/* 底部说明：与主按钮留出呼吸感，居中共两行 */
.login-card__hint {
  margin: 16px 0 0;
  max-width: 340px;
  font-size: var(--font-caption);
  line-height: 1.55;
  color: var(--ink-400);
  text-align: center;
}

/* 授权中面板 */
.authorizing-pane {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.authorizing-pane__bar {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  width: 100%;
  height: 46px;
  padding: 0 var(--space-lg);
  background-color: var(--brand-50);
  border-radius: var(--radius-lg);
}

.authorizing-pane__text {
  font-size: var(--font-body);
  font-weight: var(--fw-medium);
  color: var(--brand-500);
}

.authorizing-pane__cancel {
  color: var(--ink-600);
}
.authorizing-pane__cancel:hover {
  background-color: var(--bg-fill);
}

/* ===== 登录浮层（近满窗圆角卡片，从底部滑入） ===== */

/* 遮罩：轻微压暗背景，点击空白处取消登录 */
.login-scrim {
  position: fixed;
  inset: 0;
  z-index: 20;
  background: rgba(16, 22, 38, 0.32);
  animation: login-scrim-in 200ms ease-out;
}

@keyframes login-scrim-in {
  from { opacity: 0; }
  to { opacity: 1; }
}

/* 浮层外壳：圆角、标题区由 Vue 画，原生登录页嵌在内容区 */
.login-sheet {
  position: fixed;
  z-index: 21;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: #ffffff;
  /* 贴底：下沿不留白，圆角只在上方两角（内联 style 里给具体值） */
  box-shadow:
    0 24px 64px rgba(16, 22, 38, 0.24),
    0 2px 8px rgba(16, 22, 38, 0.08);
  /* 与后端滑入动画同节奏：整卡从窗口底部滑入 */
  animation: login-sheet-in 240ms cubic-bezier(0.33, 1, 0.68, 1);
}

@keyframes login-sheet-in {
  from { transform: translateY(100%); }
  to { transform: translateY(0); }
}

.login-sheet__header {
  position: relative;
  display: flex;
  align-items: center;
  flex-shrink: 0;
  padding: 0 96px 0 24px;
  background: #ffffff;
  border-bottom: 1px solid rgba(16, 22, 38, 0.06);
}

.login-sheet__heading {
  display: flex;
  align-items: center;
  min-width: 0;
}

.login-sheet__title {
  margin: 0;
  font-size: 16px;
  font-weight: var(--fw-semibold);
  line-height: 1.35;
  color: var(--ink-900, #161b26);
}

.login-sheet__actions {
  position: absolute;
  top: 50%;
  right: 16px;
  display: flex;
  align-items: center;
  gap: 4px;
  transform: translateY(-50%);
}

/* 右上角图标按钮：无描边，仅 hover 时给底色，两者视觉重量一致 */
.login-sheet__icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  padding: 0;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--ink-500, #6b7280);
  cursor: pointer;
  transition: background-color 120ms ease-out, color 120ms ease-out;
}

.login-sheet__icon-btn:hover {
  background: rgba(16, 22, 38, 0.06);
  color: var(--ink-800, #1b2130);
}

.login-sheet__icon-btn:focus-visible {
  outline: 2px solid var(--brand-300, #9dc0fb);
  outline-offset: 1px;
}

.login-sheet__icon-btn:focus:not(:focus-visible) {
  outline: none;
}

/* 内容区：原生 webview 会覆盖在这里，仅作占位（不留内边距，页面与标题区严丝合缝）。 */
.login-sheet__content {
  flex: 1;
  min-height: 0;
  background: #ffffff;
}

/* 浮层打开时把原登录卡片压暗并禁止交互 */
.login-page--dimmed {
  pointer-events: none;
}
</style>
