<!-- 面包屑导航 -->
<script setup lang="ts">
import { useFileBrowserStore } from "@/stores/fileBrowser";

const browser = useFileBrowserStore();
</script>

<template>
  <div class="breadcrumb">
    <template v-for="(seg, i) in browser.pathStack" :key="i">
      <span class="crumb__sep" v-if="i > 0">›</span>
      <span
        class="crumb"
        :class="{ 'is-current': i === browser.pathStack.length - 1 }"
        @click="i < browser.pathStack.length - 1 && browser.jumpTo(i)"
      >{{ seg.name }}</span>
    </template>
  </div>
</template>

<style scoped>
.breadcrumb {
  height: var(--breadcrumb-height);
  display: flex;
  align-items: center;
  padding: 0 20px;
  gap: 6px;
  background-color: var(--bg-card);
  overflow-x: auto;
  white-space: nowrap;
  flex-shrink: 0;
}

.crumb {
  font-size: var(--font-body-sm);
  color: var(--ink-400);
  cursor: pointer;
}
.crumb:hover {
  color: var(--brand-500);
}
.crumb.is-current {
  color: var(--ink-900);
  font-weight: var(--fw-semibold);
  cursor: default;
}

.crumb__sep {
  font-size: var(--font-caption);
  color: var(--ink-300);
}
</style>
