<!-- 侧边栏目录树节点，递归自引用 -->
<script setup lang="ts">
import { ref, onMounted, watch } from "vue";
import { MateIcon, MateCircularProgress } from "@/components/mate";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { useSyncStore } from "@/stores/sync";
import * as driveApi from "@/api/drive";
import type { DriveFile } from "@/api/drive";
import type { FolderLocation } from "@/stores/fileBrowser";

// 自引用（递归渲染子节点）
import SidebarTreeNode from "./SidebarTreeNode.vue";

// 单次路径联动允许的额外补加载次数，避免服务端快照延迟时失联，也避免无限递归
const MAX_RELINK_RETRIES = 2;

const props = withDefaults(defineProps<{
  location: FolderLocation;
  /** 含本节点的完整路径栈 */
  path: FolderLocation[];
  depth: number;
  /** 当前选中文件夹 id（高亮用） */
  activeId: string;
}>(), { depth: 0, activeId: "" });

const browser = useFileBrowserStore();
const sync = useSyncStore();

const expanded = ref(props.depth === 0);
const loading = ref(false);
const children = ref<DriveFile[]>([]);
// 加载请求序号：并发 loadChildren 时只有最新请求的结果能写入 children，防止旧请求覆盖新结果
const loadToken = ref(0);
// 加载期间有路径变化未处理：原请求完成后据此强制重检，避免空结果吞掉路径变化
const pendingRelink = ref(false);
// 当前路径目标缺失时已经执行的额外补加载次数
const relinkRetryCount = ref(0);

/**
 * 根节点默认展开并预加载子文件夹；非根节点挂载时尝试联动到当前路径，
 * 覆盖深层异步加载时序（子节点在 current.id 变化后才挂载）。
 */
onMounted(() => {
  if (props.depth === 0) {
    loadChildren();
  } else {
    syncExpandToCurrent();
  }
});

/**
 * 监听侧边栏刷新计数器：每次递增时重新加载已展开节点的子目录。
 */
watch(() => sync.sidebarRefresh, () => {
  if (expanded.value) {
    relinkRetryCount.value = 0;
    loadChildren();
  }
});

/**
 * 监听当前路径变化，联动展开到目标层级。
 * 覆盖「深层子节点在 current.id 变化后才挂载」的时序缺口（onMounted 也调用同一逻辑）。
 */
watch(() => browser.current.id, () => {
  relinkRetryCount.value = 0;
  syncExpandToCurrent();
});

/**
 * 加载本节点子目录。并发安全用 loadToken 序号保证：只有最新请求的结果能写入 children，
 * 旧请求（乱序返回的空列表等）被丢弃，防止覆盖。加载完成后重检联动——若新快照仍缺
 * 目标子目录则再次加载，确保深层节点最终可见。
 *
 * @param relink - 加载成功后是否执行常规联动检查；目标仍缺失时不受此参数限制，按上限补加载。
 */
async function loadChildren(relink = true): Promise<void> {
  loading.value = true;
  // 本轮请求的序号；并发时旧请求返回后比对发现非最新，丢弃结果不覆盖
  const token = ++loadToken.value;
  // 加载是否成功完成（区分空目录与加载失败）
  let loaded = false;
  try {
    const all = await driveApi.listFiles(props.location.id || undefined);
    // 旧请求丢弃：较新的 loadChildren 已发起，本结果可能过期
    if (token !== loadToken.value) return;
    const folders = all.filter(driveApi.isFolder);
    children.value = folders;
    loaded = true;
  } catch (e) {
    // 旧请求的失败同样丢弃
    if (token !== loadToken.value) return;
    children.value = [];
  } finally {
    // 仅最新请求负责清除 loading（旧请求丢弃时不干涉 loading 状态）
    if (token === loadToken.value) {
      loading.value = false;
    }
  }
  // 加载期间新发生的路径变化必须优先消费，不受原请求 relink 参数限制；
  // 否则 loadChildren(false) 执行期间设置的 pendingRelink 会被永久遗留。
  if (pendingRelink.value) {
    pendingRelink.value = false;
    syncExpandToCurrent();
    return;
  }
  if (relink && loaded && children.value.length > 0) {
    syncExpandToCurrent();
    return;
  }
  if (!isNextPathTargetMissing()) {
    relinkRetryCount.value = 0;
    return;
  }
  if (relinkRetryCount.value < MAX_RELINK_RETRIES) {
    relinkRetryCount.value++;
    await loadChildren(false);
  }
}

/**
 * 获取当前节点在路径栈中的下一级目标目录 id；当前节点已是路径末端时返回 null。
 */
function nextPathTargetId(): string | null {
  // 本节点在当前路径栈中的位置
  const idx = browser.pathStack.findIndex((loc) => loc.id === props.location.id);
  return idx >= 0 && idx + 1 < browser.pathStack.length
    ? browser.pathStack[idx + 1].id
    : null;
}

/**
 * 判断当前目录快照是否仍缺少路径栈要求的下一级目标。
 */
function isNextPathTargetMissing(): boolean {
  // 当前路径要求展开的下一级目标目录 id
  const nextId = nextPathTargetId();
  return nextId !== null && !children.value.some((child) => child.id === nextId);
}

/**
 * 若本节点是当前路径的祖先（或自身）则展开并加载子节点，使目录树联动到目标层级。
 * 供 onMounted 与路径 watch 共用，函数声明提升保证 onMounted 回调运行时可用。
 */
function syncExpandToCurrent(): void {
  // 根节点（depth 0）无条件展开并加载；其余节点需在当前路径栈中。
  const onPath = props.depth === 0
    || browser.pathStack.some((loc) => loc.id === props.location.id);
  if (!onPath) return;
  if (!expanded.value) {
    expanded.value = true;
  }
  if (loading.value) {
    // 加载期间路径变化不能丢弃：标记待重检，原请求完成后据此强制重检，
    // 避免空结果因 children.length===0 跳过联动导致目标节点永久不展开。
    pendingRelink.value = true;
    return;
  }
  // 已展开且 children 非空时，仍需检查当前路径的下一级目标是否在缓存中：
  // 缓存可能陈旧（缺新目录），右侧进入后左侧找不到目标节点，此时重新加载。
  const needReload = children.value.length === 0 || isNextPathTargetMissing();
  if (needReload) {
    // 联动触发的加载关闭常规重检；若目标仍缺失，loadChildren 会按上限补加载
    loadChildren(false);
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
