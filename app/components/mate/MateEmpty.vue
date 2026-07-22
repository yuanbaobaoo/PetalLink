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
  gap: 10px;
  padding: 48px var(--space-xl);
  text-align: center;
}
/* v2：72px 渐变圆角 badge */
.mate-empty__icon-wrap {
  width: 72px;
  height: 72px;
  border-radius: 14px;
  background: var(--grad-brand-soft);
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: var(--space-xs);
}
.mate-empty__icon {
  color: var(--brand-400);
}
.mate-empty__title {
  font-size: 15px;
  font-weight: var(--fw-semibold);
  color: var(--ink-700);
}
.mate-empty__desc {
  font-size: var(--font-body-sm);
  color: var(--ink-400);
  line-height: 1.55;
}
.mate-empty__action {
  margin-top: var(--space-sm);
}
</style>
