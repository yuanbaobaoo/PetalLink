<!-- 统计芯片，图标 + 标签 + 数值 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

withDefaults(defineProps<{
  icon: string;
  count: number;
  label: string;
}>(), {});
</script>

<template>
  <span class="mate-stat-chip">
    <MateIcon :name="icon" :size="12" />
    {{ count }} {{ label }}
  </span>
</template>

<style scoped>
.mate-stat-chip {
  display: inline-flex;
  align-items: center;
  gap: var(--space-xs);
  padding: var(--space-xs) var(--space-sm);
  border-radius: var(--radius-sm);
  background-color: var(--bg-fill);
  font-size: var(--font-caption);
  font-weight: var(--fw-medium);
  color: var(--ink-600);
  white-space: nowrap;
}
</style>
