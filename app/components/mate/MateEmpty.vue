<!-- 空状态占位 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

withDefaults(defineProps<{
  /** 图标 icon-name（默认 inbox → 用 list 近似） */
  icon?: string;
  title: string;
  description?: string;
}>(), {
  icon: "list",
  title: "",
  description: "",
});
</script>

<template>
  <div class="mate-empty">
    <div class="mate-empty__icon-wrap">
      <MateIcon :name="icon" :size="32" class="mate-empty__icon" />
    </div>
    <div class="mate-empty__title">{{ title }}</div>
    <div v-if="description" class="mate-empty__desc">{{ description }}</div>
    <div v-if="$slots.action" class="mate-empty__action">
      <slot name="action" />
    </div>
  </div>
</template>

<style scoped>
.mate-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--space-xxl);
  text-align: center;
}
.mate-empty__icon-wrap {
  width: 64px;
  height: 64px;
  border-radius: 50%;
  background-color: var(--bg-hover);
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: var(--space-lg);
}
.mate-empty__icon {
  color: var(--text-placeholder);
}
.mate-empty__title {
  font-size: var(--font-body);
  font-weight: var(--fw-medium);
  color: var(--text-primary);
}
.mate-empty__desc {
  margin-top: var(--space-sm);
  font-size: var(--font-caption);
  color: var(--text-secondary);
  line-height: 1.5;
}
.mate-empty__action {
  margin-top: var(--space-lg);
}
</style>
