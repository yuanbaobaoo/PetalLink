<!-- 竖分隔线 -->
<script setup lang="ts">
withDefaults(defineProps<{ height?: number }>(), { height: 20 });
</script>

<template>
  <div class="mate-vsep" :style="{ height: `${height}px` }" />
</template>

<style scoped>
.mate-vsep {
  width: 1px;
  background-color: var(--border);
  flex-shrink: 0;
}
</style>
