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
  padding: 0 var(--space-lg);
  gap: var(--space-xs);
  border-bottom: 0.5px solid var(--border);
  background-color: var(--bg-container);
  overflow-x: auto;
  white-space: nowrap;
  flex-shrink: 0;
}

.crumb {
  font-size: var(--font-body-sm);
  color: var(--text-secondary);
  cursor: pointer;
}
.crumb:hover {
  color: var(--color-brand);
}
.crumb.is-current {
  color: var(--text-primary);
  font-weight: var(--fw-medium);
  cursor: default;
}

.crumb__sep {
  font-size: 11px;
  color: var(--text-placeholder);
}
</style>
