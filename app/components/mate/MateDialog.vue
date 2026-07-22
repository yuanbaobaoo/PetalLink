<!-- 对话框，teleport 到 body，支持标题图标和危险样式 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

const props = withDefaults(defineProps<{
  open: boolean;
  title?: string;
  /** 标题图标 icon-name */
  titleIcon?: string;
  danger?: boolean;
  /** 点击遮罩是否关闭 */
  closeOnOverlay?: boolean;
  width?: number;
}>(), {
  title: "",
  titleIcon: "",
  danger: false,
  closeOnOverlay: true,
  width: 420,
});

const emit = defineEmits<{
  (e: "update:open", v: boolean): void;
  (e: "close"): void;
}>();

function close(): void {
  emit("update:open", false);
  emit("close");
}function onOverlay(): void {
  if (props.closeOnOverlay) close();
}
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="mate-dialog-overlay" @click.self="onOverlay">
      <div class="mate-dialog" :class="{ 'is-danger': danger }" :style="{ maxWidth: `${width}px` }">
        <div v-if="title || titleIcon || $slots.header" class="mate-dialog__header">
          <slot name="header">
            <MateIcon v-if="titleIcon" :name="titleIcon" :size="20" class="mate-dialog__title-icon" />
            <span v-if="title" class="mate-dialog__title">{{ title }}</span>
          </slot>
        </div>
        <div class="mate-dialog__body">
          <slot />
        </div>
        <div v-if="$slots.footer" class="mate-dialog__footer">
          <slot name="footer" />
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.mate-dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(28, 28, 30, 0.36);
  backdrop-filter: blur(3px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}
.mate-dialog {
  width: 100%;
  background-color: var(--bg-card);
  border-radius: var(--radius-xl);
  box-shadow: var(--sh-pop);
  display: flex;
  flex-direction: column;
  animation: dialog-fade-in 0.15s ease-out;
}
.mate-dialog__header {
  display: flex;
  align-items: center;
  gap: var(--space-md);
  padding: var(--space-xl) var(--space-xl) var(--space-sm);
}
.mate-dialog__title-icon { color: var(--brand-500); flex-shrink: 0; }
.mate-dialog.is-danger .mate-dialog__title-icon { color: var(--err); }
.mate-dialog__title {
  font-size: 17px;
  font-weight: var(--fw-semibold);
  color: var(--ink-900);
}
.mate-dialog__body {
  padding: var(--space-sm) var(--space-xl) 20px;
  font-size: var(--font-body);
  line-height: 1.65;
  color: var(--ink-600);
}
.mate-dialog__footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 0 var(--space-xl) 20px;
}
</style>
