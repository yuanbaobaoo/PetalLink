<!-- 页面脚手架：AppBar + 内容区 + 底部安全区 -->
<script setup lang="ts">
withDefaults(defineProps<{ /** 顶部栏高度占位（可选） */ flush?: boolean }>(), { flush: false });
</script>

<template>
  <div class="mate-scaffold" :class="{ 'mate-scaffold--flush': flush }">
    <slot />
  </div>
</template>

<style scoped>
.mate-scaffold {
  width: 100%;
  height: 100%;
  background-color: var(--bg-page);
  display: flex;
  flex-direction: column;
}
.mate-scaffold--flush {
  background-color: var(--bg-container);
}
</style>
