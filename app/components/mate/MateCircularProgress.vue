<!-- 环形进度条，确定/不确定两种态 -->
<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(defineProps<{
  size?: number;
  strokeWidth?: number;
  color?: string;
  /** 0.0-1.0；null = 不确定 */
  value?: number | null;
}>(), {
  size: 24,
  strokeWidth: 2.5,
  color: "var(--color-brand)",
  value: null,
});

// SVG 半径
const radius = computed(() => (props.size - props.strokeWidth) / 2);
// SVG 周长
const circumference = computed(() => 2 * Math.PI * radius.value);
const isIndeterminate = computed(() => props.value === null || props.value === undefined);

// 不确定态：画约 1.5 弧度（≈86°）的弧
const indeterminateDash = computed(() => {
  const arc = 1.5; // 弧度
  return (arc / (2 * Math.PI)) * circumference.value;
});
// 不确定态间隔
const indeterminateGap = computed(() => circumference.value - indeterminateDash.value);

// dash 偏移量
const dashOffset = computed(() => {
  if (isIndeterminate.value) return circumference.value / 4; // 起笔顶部
  const v = Math.max(0, Math.min(1, props.value as number));
  return circumference.value * (1 - v);
});
// dash 数组
const dashArray = computed(() =>
  isIndeterminate.value
    ? `${indeterminateDash.value} ${indeterminateGap.value}`
    : `${circumference.value}`
);
</script>

<template>
  <svg
    class="mate-circular-progress"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
  >
    <!-- 轨道 -->
    <circle
      cx="12"
      cy="12"
      :r="radius * (24 / size)"
      :stroke-width="strokeWidth * (24 / size)"
      stroke="var(--bg-active)"
    />
    <!-- 填充弧 -->
    <circle
      cx="12"
      cy="12"
      :r="radius * (24 / size)"
      :stroke-width="strokeWidth * (24 / size)"
      :stroke="color"
      stroke-linecap="round"
      :stroke-dasharray="dashArray"
      :stroke-dashoffset="dashOffset * (24 / size)"
      transform="rotate(-90 12 12)"
    />
  </svg>
</template>

<style scoped>
.mate-circular-progress {
  display: inline-block;
  vertical-align: middle;
  flex-shrink: 0;
}
</style>
