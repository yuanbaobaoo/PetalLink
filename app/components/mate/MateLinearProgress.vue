<!-- 线性进度条（v2：渐变填充 + 灰底轨道），确定/不确定两种态 -->
<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(defineProps<{
  /** 0.0-1.0；null = 不确定 */
  value?: number | null;
  height?: number;
  /** 填充背景（支持渐变，默认品牌渐变） */
  color?: string;
}>(), {
  value: null,
  height: 6,
  color: "var(--grad-brand)",
});

const isIndeterminate = computed(() => props.value === null || props.value === undefined);
// 填充宽度百分比
const fillWidth = computed(() => {
  if (isIndeterminate.value) return "100%";
  const v = Math.max(0, Math.min(1, props.value as number));
  return `${v * 100}%`;
});
// 圆角半径
const radius = computed(() => `${props.height / 2}px`);
</script>

<template>
  <div class="mate-linear-progress" :style="{ height: `${props.height}px`, borderRadius: radius }">
    <div
      v-if="isIndeterminate"
      class="mate-linear-progress__indeterminate"
      :style="{ background: color, borderRadius: radius }"
    />
    <div
      v-else
      class="mate-linear-progress__fill"
      :style="{ width: fillWidth, background: color, borderRadius: radius }"
    />
  </div>
</template>

<style scoped>
.mate-linear-progress {
  width: 100%;
  background-color: var(--bg-fill);
  overflow: hidden;
  position: relative;
}
.mate-linear-progress__fill {
  height: 100%;
  transition: width 0.3s ease;
}
/* 不确定态：30% 宽指示器从左到右往复 */
.mate-linear-progress__indeterminate {
  position: absolute;
  top: 0;
  left: 0;
  width: 30%;
  height: 100%;
  animation: mate-linear-indeterminate 1.2s ease-in-out infinite;
}
@keyframes mate-linear-indeterminate {
  0% { left: -30%; }
  50% { left: 100%; }
  100% { left: 100%; }
}
</style>
