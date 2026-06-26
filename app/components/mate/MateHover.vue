<!-- 悬停探测器，作用域插槽暴露 hovered 状态 -->
<script setup lang="ts">
import { ref } from "vue";

const hovered = ref(false);

const props = withDefaults(defineProps<{ cursor?: "pointer" | "default" }>(), { cursor: "pointer" });
</script>

<template>
  <div
    class="mate-hover"
    :style="{ cursor: props.cursor === 'pointer' ? 'pointer' : 'default' }"
    @mouseenter="hovered = true"
    @mouseleave="hovered = false"
  >
    <slot :hovered="hovered" />
  </div>
</template>

<style scoped>
.mate-hover {
  display: inline-block;
}
</style>
