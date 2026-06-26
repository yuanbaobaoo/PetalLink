<!-- 标签/徽章，多尺寸多主题 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

export type TagTheme = "default" | "primary" | "success" | "warning" | "error";
export type TagSize = "small" | "medium";

withDefaults(defineProps<{
  label: string;
  theme?: TagTheme;
  size?: TagSize;
  /** 图标 icon-name */
  icon?: string;
}>(), {
  theme: "default",
  size: "medium",
  icon: "",
});
</script>

<template>
  <span :class="['mate-tag', `mate-tag--${theme}`, `mate-tag--${size}`]">
    <MateIcon v-if="icon" :name="icon" :size="size === 'small' ? 12 : 14" />
    {{ label }}
  </span>
</template>

<style scoped>
.mate-tag {
  display: inline-flex;
  align-items: center;
  gap: var(--space-xs);
  border: 1px solid;
  border-radius: var(--radius-sm);
  font-weight: var(--fw-medium);
  white-space: nowrap;
  flex-shrink: 0;
}

.mate-tag--small {
  padding: 2px var(--space-sm);
  font-size: var(--font-caption);
}
.mate-tag--medium {
  padding: var(--space-xs) var(--space-md);
  font-size: var(--font-body-sm);
}

/* default：灰底 */
.mate-tag--default {
  background-color: var(--bg-hover);
  color: var(--color-brand);
  border-color: var(--border);
}
.mate-tag--primary {
  background-color: var(--color-brand-light);
  color: var(--color-brand);
  border-color: var(--color-brand);
}
.mate-tag--success {
  background-color: var(--color-success-bg);
  color: var(--color-success);
  border-color: var(--color-success);
}
.mate-tag--warning {
  background-color: var(--color-warning-bg);
  color: var(--color-warning);
  border-color: var(--color-warning);
}
.mate-tag--error {
  background-color: var(--color-error-bg);
  color: var(--color-error);
  border-color: var(--color-error);
}
</style>
