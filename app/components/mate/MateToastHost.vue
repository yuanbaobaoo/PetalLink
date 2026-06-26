<!-- Toast 宿主，底部居中，teleport 到 body -->
<script setup lang="ts">
import { toasts } from "./useToast";
import type { ToastVariant } from "./useToast";

// 各变体背景色映射
const bg: Record<ToastVariant, string> = {
  default: "var(--gray-13)",
  success: "var(--color-success)",
  warning: "var(--color-warning)",
  error: "var(--color-error)",
};
</script>

<template>
  <Teleport to="body">
    <div class="mate-toast-host">
      <div
        v-for="t in toasts"
        :key="t.id"
        class="mate-toast"
        :style="{ backgroundColor: bg[t.variant] }"
      >
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
  padding: 10px var(--space-lg);
  border-radius: var(--radius-sm);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
  font-size: var(--font-body);
  color: #fff;
  text-align: center;
  animation: toast-in 0.2s ease-out;
}
</style>
