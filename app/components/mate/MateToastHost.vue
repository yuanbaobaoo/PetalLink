<!-- Toast 宿主（v2：近黑毛玻璃浮丸），底部居中，teleport 到 body -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";
import { toasts } from "./useToast";
import type { ToastVariant } from "./useToast";

// 各变体图标映射（default 无图标）
const icon: Record<ToastVariant, string> = {
  default: "",
  success: "check",
  warning: "alert",
  error: "x",
};
</script>

<template>
  <Teleport to="body">
    <div class="mate-toast-host">
      <div
        v-for="t in toasts"
        :key="t.id"
        class="mate-toast"
        :class="`mate-toast--${t.variant}`"
      >
        <MateIcon v-if="icon[t.variant]" :name="icon[t.variant]" :size="16" class="mate-toast__icon" />
        {{ t.message }}
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.mate-toast-host {
  position: fixed;
  left: 0;
  right: 0;
  bottom: 48px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-sm);
  z-index: 2000;
  pointer-events: none;
}
.mate-toast {
  max-width: 480px;
  display: inline-flex;
  align-items: center;
  gap: var(--space-sm);
  padding: 10px 18px;
  border-radius: var(--radius-lg);
  background: rgba(28, 28, 30, 0.92);
  backdrop-filter: blur(8px);
  box-shadow: var(--sh-lg);
  font-size: var(--font-body-sm);
  font-weight: var(--fw-medium);
  color: #fff;
  animation: toast-in 0.2s ease-out;
}
.mate-toast--success .mate-toast__icon { color: #4ade80; }
.mate-toast--warning .mate-toast__icon { color: #fbbf24; }
.mate-toast--error .mate-toast__icon { color: #fb7185; }
</style>
