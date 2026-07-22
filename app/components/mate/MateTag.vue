<!-- 标签 chip（v2：无底框、小圆角），多尺寸多主题 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

export type TagTheme = "default" | "primary" | "success" | "warning" | "error" | "info";
export type TagSize = "small" | "medium";

withDefaults(defineProps<{
  label: string;
  theme?: TagTheme;
  size?: TagSize;
  /** 图标 icon-name */
  icon?: string;
  /** 描边可点变体（v2 chip--outline） */
  outline?: boolean;
  /** 选中态（仅 outline 变体） */
  active?: boolean;
}>(), {
  theme: "default",
  size: "medium",
  icon: "",
  outline: false,
  active: false,
});
</script>

<template>
  <span
    :class="[
      'mate-tag',
      `mate-tag--${theme}`,
      `mate-tag--${size}`,
      { 'is-outline': outline, 'is-active': active },
    ]"
  >
    <MateIcon v-if="icon" :name="icon" :size="size === 'small' ? 12 : 14" />
    {{ label }}
  </span>
</template>

<style scoped>
.mate-tag {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-radius: var(--radius-sm);
  font-weight: var(--fw-medium);
  white-space: nowrap;
  flex-shrink: 0;
}

.mate-tag--small {
  height: 20px;
  padding: 0 var(--space-sm);
  font-size: var(--font-caption);
}
.mate-tag--medium {
  height: 24px;
  padding: 0 10px;
  font-size: var(--font-caption);
}

/* default：灰底 */
.mate-tag--default {
  background-color: var(--bg-fill);
  color: var(--ink-600);
}
.mate-tag--primary {
  background-color: var(--brand-50);
  color: var(--brand-500);
}
.mate-tag--success {
  background-color: var(--ok-bg);
  color: var(--ok);
}
.mate-tag--warning {
  background-color: var(--warn-bg);
  color: var(--warn);
}
.mate-tag--error {
  background-color: var(--err-bg);
  color: var(--err);
}
.mate-tag--info {
  background-color: var(--info-bg);
  color: var(--info);
}

/* 描边可点变体（日志过滤 chip 等场景） */
.mate-tag.is-outline {
  background-color: var(--bg-card);
  box-shadow: inset 0 0 0 1.5px var(--line-strong);
  color: var(--ink-600);
  cursor: pointer;
}
.mate-tag.is-outline.is-active {
  box-shadow: inset 0 0 0 1.5px var(--brand-500);
  color: var(--brand-500);
  background-color: var(--brand-50);
}
</style>
