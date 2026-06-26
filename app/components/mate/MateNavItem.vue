<!-- 导航项，左侧导航栏用 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

withDefaults(defineProps<{
  label: string;
  /** 图标 icon-name */
  icon?: string;
  active?: boolean;
  /** 左侧缩进 px（层级用） */
  indent?: number;
  height?: number;
}>(), {
  icon: "",
  active: false,
  indent: 0,
  height: 32,
});

defineEmits<{ (e: "click"): void }>();
</script>

<template>
  <div
    :class="['mate-nav-item', { 'is-active': active }]"
    :style="{ paddingLeft: `${indent + 8}px`, height: `${height}px` }"
    @click="$emit('click')"
  >
    <MateIcon v-if="icon" :name="icon" :size="16" class="mate-nav-item__icon" />
    <span class="mate-nav-item__label">{{ label }}</span>
    <slot />
  </div>
</template>

<style scoped>
.mate-nav-item {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding-right: var(--space-sm);
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--text-primary);
  font-size: var(--font-body);
  font-weight: var(--fw-regular);
  transition: background-color 0.15s;
}
.mate-nav-item:hover {
  background-color: var(--bg-hover);
}
.mate-nav-item.is-active {
  background-color: var(--color-brand-lighter);
  color: var(--color-brand);
  font-weight: var(--fw-medium);
}
.mate-nav-item__icon {
  flex-shrink: 0;
  color: var(--text-secondary);
}
.mate-nav-item.is-active .mate-nav-item__icon {
  color: var(--color-brand);
}
.mate-nav-item__label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
