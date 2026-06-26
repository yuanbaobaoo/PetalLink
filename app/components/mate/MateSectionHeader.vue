<!-- 分区标题，图标 + 标题 + 可选尾部，底部 1px 分隔 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

withDefaults(defineProps<{
  text: string;
  /** 图标 icon-name */
  icon?: string;
}>(), { icon: "" });
</script>

<template>
  <div class="mate-section-header">
    <MateIcon v-if="icon" :name="icon" :size="18" class="mate-section-header__icon" />
    <span class="mate-section-header__text">{{ text }}</span>
    <div v-if="$slots.trailing" class="mate-section-header__trailing">
      <slot name="trailing" />
    </div>
  </div>
</template>

<style scoped>
.mate-section-header {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding-bottom: var(--space-md);
  border-bottom: 1px solid var(--border);
  margin-bottom: var(--space-xxl);
}
.mate-section-header__icon {
  color: var(--color-brand);
  flex-shrink: 0;
}
.mate-section-header__text {
  font-size: var(--font-title-sm);
  font-weight: var(--fw-semibold);
  color: var(--text-primary);
}
.mate-section-header__trailing {
  margin-left: auto;
}
</style>
