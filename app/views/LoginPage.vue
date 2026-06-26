<!--
  登录页 —— macOS 窗口 + 居中品牌卡片。

  布局：
  - 渐变背景（135deg #EBF1FF → #F5F5F5 → #FFFFFF）
  - 3 个低透明度装饰圆
  - 居中卡片（max-width 480，radius 9）
  - 卡片内：品牌容器图标 + 标题 + 品牌分隔线 + 可选 warning/error banner + 主按钮
  - 授权中：替换为 spinner + 提示 + 取消按钮
-->
<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useAuthStore } from "@/stores/auth";
import { MateButton, MateAppLogo, MateInfoBanner, MateCircularProgress } from "@/components/mate";

const auth = useAuthStore();

// 应用完整标题
const appTitle = "PetalLink - 华为云盘客户端开源版";

// client_secret 未配置时的提示文案
const secretWarning =
  "尚未配置 client_secret。请在项目根目录创建 .env 文件，写入：\nHWCLOUD_CLIENT_SECRET=<你的 64 位 hex>\n（参考 .env.example；或在启动命令中 --dart-define=HWCLOUD_CLIENT_SECRET=...）";

// 登录按钮是否可点（secret 已配置且非加载中）
const canLogin = computed(
  () => auth.secretConfigured && !auth.loading
);

const showAuthorizing = computed(
  () => auth.loading && auth.status === "authorizing"
);

/**
 * 启动时恢复登录态 + 检查 secret
 */
onMounted(() => {
  auth.restore();
});

function handleLogin(): void {
  auth.login();
}

function handleCancel(): void {
  auth.cancelLogin();
}

function handleDismissError(): void {
  auth.dismissError();
}
</script>

<template>
  <div class="login-page">
    <!-- 装饰圆（低透明度品牌色） -->
    <div class="decor-circle decor-circle--lg" />
    <div class="decor-circle decor-circle--md" />
    <div class="decor-circle decor-circle--sm" />

    <!-- 居中卡片 -->
    <div class="login-card">
      <!-- 品牌容器图标 -->
      <MateAppLogo container text="" />

      <!-- 标题 -->
      <h1 class="login-card__title">{{ appTitle }}</h1>

      <!-- 品牌分隔线 -->
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
            <span class="authorizing-pane__text">请在浏览器中完成授权...</span>
          </div>
          <MateButton variant="text" icon="x" @click="handleCancel">
            取消授权
          </MateButton>
        </div>

        <!-- 登录按钮 -->
        <MateButton
          v-else
          variant="primary"
          icon="cloud"
          full-width
          :height="40"
          :disabled="!canLogin"
          @click="handleLogin"
        >
          使用华为账号登录
        </MateButton>
      </div>

      <!-- 底部说明 -->
      <p class="login-card__hint">点击后将打开浏览器，支持账号密码或手机扫码登录</p>
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
  /* 渐变背景 */
  background: linear-gradient(
    135deg,
    #ebf1ff 0%,
    #f5f5f5 50%,
    #ffffff 100%
  );
}

/* 装饰圆（品牌色低透明度） */
.decor-circle {
  position: absolute;
  border-radius: 50%;
  pointer-events: none;
  background-color: var(--color-brand);
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
  opacity: 0.04;
}

/* 登录卡片 */
.login-card {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
  max-width: 480px;
  padding: var(--space-xxl) var(--space-xl);
  /*background-color: var(--bg-container);*/
  border-radius: var(--radius-lg);
  /*border: 0.5px solid var(--border);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.16),
    0 0 0 0.5px rgba(0, 0, 0, 0.06);*/
}

.login-card__title {
  margin-top: var(--space-md);
  font-size: var(--font-title-md);
  font-weight: var(--fw-semibold);
  color: var(--text-primary);
  letter-spacing: -0.2px;
  text-align: center;
}

.login-card__divider {
  width: 40px;
  height: 2px;
  margin: var(--space-xs) 0;
  background-color: var(--color-brand);
  border-radius: 1px;
}

.login-card__banner {
  width: 100%;
  margin-top: var(--space-md);
}

.login-card__actions {
  width: 100%;
  margin-top: var(--space-xl);
}

.login-card__hint {
  margin-top: var(--space-md);
  font-size: var(--font-caption);
  color: var(--text-secondary);
  text-align: center;
}

/* 授权中面板 */
.authorizing-pane {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-sm);
}

.authorizing-pane__bar {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-sm);
  width: 100%;
  height: 40px;
  padding: 0 var(--space-lg);
  background-color: var(--color-brand-lighter);
  border-radius: var(--radius-sm);
}

.authorizing-pane__text {
  font-size: var(--font-body);
  font-weight: var(--fw-medium);
  color: var(--color-brand);
}
</style>
