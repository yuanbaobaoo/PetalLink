<!-- 侧边栏目录树节点，递归自引用 -->
<script setup lang="ts">
import { ref, onMounted } from "vue";
import { MateIcon, MateCircularProgress } from "@/components/mate";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import * as driveApi from "@/api/drive";
import type { DriveFile } from "@/api/drive";
import type { FolderLocation } from "@/stores/fileBrowser";

// 自引用（递归渲染子节点）
import SidebarTreeNode from "./SidebarTreeNode.vue";

const props = withDefaults(defineProps<{
  location: FolderLocation;
  /** 含本节点的完整路径栈 */
  path: FolderLocation[];
  depth: number;
  /** 当前选中文件夹 id（高亮用） */
  activeId: string;
}>(), { depth: 0, activeId: "" });

const browser = useFileBrowserStore();

const expanded = ref(props.depth === 0);
const loading = ref(false);
const children = ref<DriveFile[]>([]);

/**
 * 根节点默认展开并预加载子文件夹
 */
onMounted(() => {
  if (props.depth === 0) loadChildren();
});

async function loadChildren(): Promise<void> {
  loading.value = true;
  try {
    const all = await driveApi.listFiles(props.location.id || undefined);
    const folders = all.filter(driveApi.isFolder);
    console.log("[SidebarTreeNode] loadChildren:", props.location.name, "->", all.length, "items,", folders.length, "folders");
    children.value = folders;
  } catch (e) {
    console.error("[SidebarTreeNode] loadChildren error:", e);
    children.value = [];
  } finally {
    loading.value = false;
  }
}

async function handleToggleExpand(event: Event): Promise<void> {
  event.stopPropagation();
  expanded.value = !expanded.value;
  if (expanded.value && children.value.length === 0 && !loading.value) {
    await loadChildren();
  }
}

function handleNavigate(): void {
  browser.pathStack = [...props.path];
  browser.loadCurrent();
}
</script>

<template>
  <div>
    <div
      class="tree-node"
      :class="{ 'is-active': browser.current.id === location.id }"
      :style="{ '--tree-indent': `${depth * 14 + 8}px` }"
      @click="handleNavigate"
    >
      <span v-if="loading" class="tree-chevron"><MateCircularProgress :size="12" :stroke-width="2" /></span>
      <span
        v-else
        class="tree-chevron"
        :class="{ 'is-expanded': expanded }"
        @click="handleToggleExpand"
      >
        <MateIcon name="arrow" :size="12" />
      </span>
      <MateIcon name="folder" :size="16" class="tree-node__icon" />
      <span class="tree-node__name">{{ location.name }}</span>
    </div>
    <!-- 递归渲染子节点 -->
    <template v-if="expanded">
      <SidebarTreeNode
        v-for="child in children"
        :key="child.id"
        :location="{ id: child.id, name: child.name }"
        :path="[...path, { id: child.id, name: child.name }]"
        :depth="depth + 1"
        :active-id="activeId"
      />
    </template>
  </div>
</template>

<style scoped>
.tree-node {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  height: 28px;
  padding-left: var(--tree-indent, 8px);
  padding-right: var(--space-sm);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background-color 0.1s;
  font-size: var(--font-body-sm);
  color: var(--text-secondary);
}
.tree-node:hover { background-color: var(--bg-hover); }
.tree-node.is-active {
  background-color: var(--color-brand-lighter);
  color: var(--color-brand);
  font-weight: var(--fw-medium);
}
.tree-chevron {
  width: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  cursor: pointer;
  color: var(--text-secondary);
  transition: transform 0.15s ease;
}
.tree-chevron.is-expanded { transform: rotate(90deg); }
.tree-node__icon { color: var(--color-brand); flex-shrink: 0; }
.tree-node__name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
