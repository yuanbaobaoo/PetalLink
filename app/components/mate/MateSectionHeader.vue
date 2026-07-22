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
  gap: 10px;
  margin-bottom: 20px;
}
.mate-section-header__icon {
  color: var(--brand-500);
  flex-shrink: 0;
}
.mate-section-header__text {
  font-size: 18px;
  font-weight: var(--fw-semibold);
  color: var(--ink-900);
}
.mate-section-header__trailing {
  margin-left: auto;
}
</style>
