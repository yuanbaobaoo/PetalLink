<!-- Logo + 文字组合，PNG 失败回退纯文字 -->
<script setup lang="ts">
import { ref } from "vue";
import MateAppLogo from "./MateAppLogo.vue";

// Logo 图片是否加载失败。
const imgError = ref(false);

// 组件输入参数。
const props = withDefaults(defineProps<{ height?: number }>(), { height: 32 });
</script>

<template>
  <MateAppLogo v-if="!imgError" :size="props.height" @error="imgError = true" />
  <div v-else class="mate-logo-with-text mate-logo-with-text--fallback" :style="{ height: `${props.height}px` }">
    <span class="mate-logo-with-text__text" :style="{ fontSize: `${Math.round(props.height * 0.36)}px` }">PetalLink</span>
  </div>
</template>

<style scoped>
.mate-logo-with-text {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
}
.mate-logo-with-text--fallback {
  gap: 6px;
}
.mate-logo-with-text__text {
  font-weight: var(--fw-semibold);
  color: var(--ink-900);
}
</style>
