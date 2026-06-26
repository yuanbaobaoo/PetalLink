<!-- 旋转图标，1s 循环 -->
<script setup lang="ts">
import MateIcon from "./MateIcon.vue";

withDefaults(defineProps<{
  name: string;
  size?: number;
}>(), { size: 16 });
</script>

<template>
  <MateIcon :name="name" :size="size" spin class="mate-spinning-icon" />
</template>

<style scoped>
.mate-spinning-icon {
  color: var(--color-brand);
}
</style>
