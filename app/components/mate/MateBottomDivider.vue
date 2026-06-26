<!-- 底部分隔线 -->
<script setup lang="ts">
withDefaults(defineProps<{
  /** 底色 token（默认 bg-container） */
  background?: string;
  /** 分隔线色 token（默认 border） */
  color?: string;
}>(), {
  background: "var(--bg-container)",
  color: "var(--border)",
});
</script>

<template>
  <div class="mate-bottom-divider" :style="{ backgroundColor: background, borderBottomColor: color }">
    <slot />
  </div>
</template>

<style scoped>
.mate-bottom-divider {
  border-bottom: 0.5px solid var(--border);
}
</style>
