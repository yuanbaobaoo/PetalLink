<!-- SVG 图标，引用 IconSprite 中的 symbol -->
<script setup lang="ts">
import { computed } from "vue";

interface Props {
  /** 图标名（不带 i- 前缀），如 "cloud" / "folder" / "search" */
  name: string;
  /** 图标尺寸 px（design 默认 16，传输列表 18，侧边栏 Logo 20） */
  size?: number;
  /** 是否旋转（同步中图标用，对齐 §8 .icon-spin） */
  spin?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  size: 16,
  spin: false,
});

// :style 绑定尺寸
const sizeStyle = computed(() => ({
  width: `${props.size}px`,
  height: `${props.size}px`,
}));
</script>

<template>
  <svg
    class="mate-icon"
    :class="{ 'mate-icon--spin': spin }"
    :style="sizeStyle"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <use :href="`#i-${name}`" />
  </svg>
</template>

<style scoped>
.mate-icon {
  display: inline-block;
  vertical-align: middle;
  flex-shrink: 0;
}
.mate-icon--spin {
  animation: spin 1s linear infinite;
}
</style>
