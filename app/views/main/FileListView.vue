<!-- 文件列表，表头可拖拽列宽 + 多选 + 右键菜单 -->
<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from "vue";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { useSyncStore } from "@/stores/sync";
import { commands } from "@/api/generated";
import type { DriveFile, FreeableItem } from "@/api/generated";
import * as driveApi from "@/api/drive";
import {
  MateIcon, MateCheckbox, MateButton, MateDialog, MateTextField,
  MateCircularProgress, MateEmpty,
} from "@/components/mate";
import { confirmDialog, showToast } from "@/components/mate";
import { useAsyncAction } from "@/composables/useAsyncAction";
import { useFileOperation } from "@/composables/useFileOperation";
import { runDragImportFromDrop } from "@/composables/useDragUpload";
import { formatFileSize, formatDateTime } from "@/utils/format";
import { extractErrorMessage } from "@/utils/error";

// 同步状态文案：仅云端（未同步到本地）
const SYNC_STATUS_CLOUD_ONLY = "仅云端（未同步到本地）";
// 同步状态文案：已双端对齐，文件已下载到本地
const SYNC_STATUS_SYNCED_LOCAL = "已同步到本地";
// 同步状态文案：本地仅有占位文件，内容尚未下载
const SYNC_STATUS_PLACEHOLDER = "仅云端（尚未下载）";
// 同步状态文案：文件夹
const SYNC_STATUS_FOLDER = "文件夹";
// 单批并发缩略图请求数，避免目录内图片较多时串行阻塞或瞬时打满接口
const THUMBNAIL_BATCH_SIZE = 6;

// 文件浏览器 store
const browser = useFileBrowserStore();
// 同步全局状态 store
const sync = useSyncStore();

// 镜像根目录（下载目标用）
const mountDir = computed(() => sync.mountDir);

// 列宽（可拖拽调整）
const sizeWidth = ref(100);
// 修改时间列宽
const timeWidth = ref(150);

// 多选集合
const checked = ref<Set<string>>(new Set());
// 是否显示复选框列
const showCheckboxes = ref(false);
// 当前选中聚焦的文件 ID
const selectedId = ref<string>("");
// 批量删除异步操作防重复
const { loading: bulkDeleteLoading, run: runBulkDelete } = useAsyncAction();
// 批量下载异步操作防重复
const { loading: bulkDownloadLoading, run: runBulkDownload } = useAsyncAction();
// 批量释放空间异步操作防重复
const { loading: bulkFreeUpLoading, run: runBulkFreeUp } = useAsyncAction();
// 右键「同步」非 MateButton，仅用 run 做防重复（不绑 loading 显示）
const { run: runSyncItem } = useAsyncAction();
// 打开本地项目的互斥操作，避免重复双击同时下载或拉起多个默认应用实例
const { run: runOpenItem } = useAsyncAction();

// 文件操作统一封装：守卫 + 错误归一 + 统一通知
const fileOp = useFileOperation({
  isIndexing: () => sync.isIndexing,
  mountConfigured: () => sync.mountConfigured,
  refresh: () => browser.refresh(),
  clearSelection: () => { checked.value.clear(); showCheckboxes.value = false; },
});

// 排序字段
const sortField = ref<"name" | "size" | "modifiedTime">("name");
// 排序方向（true=升序）
const sortAsc = ref(true);

// 当前文件夹文件列表
const files = computed(() => browser.files);

// 排序后的文件列表（文件夹优先，同类型内按选定字段排序）
const sortedFiles = computed(() => [...files.value].sort((a, b) => {
  // 文件夹优先排在前面
  const aIsFolder = driveApi.isFolder(a);
  // 右侧项目是否为文件夹。
  const bIsFolder = driveApi.isFolder(b);
  if (aIsFolder && !bIsFolder) return -1;
  if (!aIsFolder && bIsFolder) return 1;
  // 同类型内按选定字段排序
  let cmp: number;
  if (sortField.value === "name") cmp = a.name.localeCompare(b.name);
  else if (sortField.value === "size") cmp = a.size - b.size;
  else cmp = (a.edited_time ?? "").localeCompare(b.edited_time ?? "");
  return sortAsc.value ? cmp : -cmp;
}));

// 已选中的文件数量
const checkedCount = computed(() => checked.value.size);

// 表头 tri-state 值：false | true | null(部分)
const headerCheck = computed<boolean | null>(() => {
  if (checkedCount.value === 0) return false;
  if (checkedCount.value === sortedFiles.value.length) return true;
  return null;
});

// 缩略图缓存（文件 ID → 图片 URL）
const thumbUrls = ref<Record<string, string>>({});

// 拖拽列宽状态
let dragStartX = 0;
// 拖拽起始列宽
let dragStartW = 0;
// 当前拖拽的列（null 表示未拖拽）
const dragging = ref<"size" | "time" | null>(null);

// 同步进度对话框：downloadOnDemand 到镜像目录
const downloading = ref<{ open: boolean; name: string }>({ open: false, name: "" });


// 右键菜单状态
const contextMenu = ref<{ show: boolean; x: number; y: number; file: DriveFile | null; canFreeUp: boolean }>({
  show: false, x: 0, y: 0, file: null, canFreeUp: false,
});
// 右键菜单 DOM 引用（用于定位钳制）
const ctxMenuEl = ref<HTMLElement | null>(null);

// 重命名对话框状态
const showRenameDialog = ref(false);
// 重命名目标文件
const renameTarget = ref<DriveFile | null>(null);
// 重命名输入值
const renameValue = ref("");

// 属性对话框状态
const showPropsDialog = ref(false);
// 属性目标文件
const propsTarget = ref<DriveFile | null>(null);

// 释放空间预览对话框状态
const showFreeUpDialog = ref(false);
// 释放空间预览候选项（用户确认后据此批量执行）
const freeUpPreviewItems = ref<FreeableItem[]>([]);
// 释放空间预览执行中标记
const freeUpConfirmLoading = ref(false);
// 释放空间预览候选项总字节数
const freeUpTotalBytes = computed(() => freeUpPreviewItems.value.reduce((sum, it) => sum + it.size, 0));

// 批量文件同步状态缓存（fileId → "synced" | "placeholder" | "not_synced" | "folder"）
const fileStatuses = ref<Record<string, string>>({});

// 外部文件拖入列表区域的高亮态
const dragImportOver = ref(false);
// 系统文件拖放事件监听句柄（卸载时释放）
let unlistenDragDrop: UnlistenFn | null = null;

/**
 * 挂载时监听系统文件拖放：HTML5 拖拽被 Tauri 拦截，必须走 onDragDropEvent 拿本地路径。
 */
onMounted(async () => {
  try {
    unlistenDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter" || event.payload.type === "over") {
        dragImportOver.value = true;
      } else if (event.payload.type === "drop") {
        dragImportOver.value = false;
        void runDragImportFromDrop(event.payload.paths);
      } else {
        // leave：拖出窗口或取消，统一收起高亮。
        dragImportOver.value = false;
      }
    });
  } catch {
    // 拖放监听不可用时静默降级，不影响列表其余功能。
  }
});

/**
 * 卸载时释放拖放监听，避免组件销毁后回调悬挂。
 */
onUnmounted(() => {
  unlistenDragDrop?.();
});

/**
 * 监听排序文件变化，自动加载缩略图和批量同步状态
 */
watch(sortedFiles, () => {
  void loadThumbs();
  void refreshBatchStatus();
});

/**
 * 批量拉取当前文件列表中所有文件的同步状态。
 * 仅在已配置挂载目录时执行。
 */
async function refreshBatchStatus(): Promise<void> {
  if (!sync.mountConfigured) return;
  // 当前列表的全部云端 ID。
  const ids = sortedFiles.value.map((f) => f.id);
  if (ids.length === 0) return;
  try {
    // 后端返回的 fileId 到本地状态映射。
    const map = await commands.syncBatchFileStatus(ids);
    fileStatuses.value = map;
  } catch {
    // 批量查询失败时清空缓存，回退到默认云朵图标
    fileStatuses.value = {};
  }
}

function getFileStatus(f: DriveFile): string {
  return fileStatuses.value[f.id] ?? "not_synced";
}

/**
 * 判断文件是否为缩略图类型（图片/视频）
 *
 * @param f - 文件对象
 */
function isThumbnailType(f: DriveFile): boolean {
  // 空 MIME 按不支持缩略图处理。
  const mime = f.mime_type ?? "";
  return mime.startsWith("image/") || mime.startsWith("video/");
}

function thumbUrl(f: DriveFile): string {
  return thumbUrls.value[f.id] ?? "";
}

/**
 * 预加载当前列表中所有文件的缩略图
 */
async function loadThumbs(): Promise<void> {
  // 当前目录内尚未缓存的图片和视频文件
  const targets = sortedFiles.value.filter(
    (file) => isThumbnailType(file) && !thumbUrls.value[file.id],
  );
  // 当前批次起始下标
  for (let index = 0; index < targets.length; index += THUMBNAIL_BATCH_SIZE) {
    // 当前限流批次
    const batch = targets.slice(index, index + THUMBNAIL_BATCH_SIZE);
    // 当前批次的缩略图结果
    const loaded = await Promise.all(batch.map(async (file) => ({
      fileId: file.id,
      url: await driveApi.getThumbnail(file.id),
    })));
    // 合并后的缩略图缓存
    const nextUrls = { ...thumbUrls.value };
    for (const item of loaded) {
      if (item.url) nextUrls[item.fileId] = item.url;
    }
    thumbUrls.value = nextUrls;
  }
}

/**
 * 开始拖拽调整列宽
 *
 * @param col - 要调整的列
 * @param e - 鼠标事件
 */
function startDrag(col: "size" | "time", e: MouseEvent): void {
  dragging.value = col; dragStartX = e.clientX;
  dragStartW = col === "size" ? sizeWidth.value : timeWidth.value;
}

/**
 * 拖拽中更新列宽
 *
 * @param e - 鼠标事件
 */
function onDrag(e: MouseEvent): void {
  if (!dragging.value) return;
  // 列宽限制在可读范围内。
  const newW = Math.max(64, Math.min(400, dragStartW + e.clientX - dragStartX));
  if (dragging.value === "size") sizeWidth.value = newW; else timeWidth.value = newW;
}

/**
 * 结束拖拽
 */
function endDrag(): void { dragging.value = null; }

/**
 * 全选/取消全选
 */
function handleToggleSelectAll(): void {
  if (checkedCount.value === sortedFiles.value.length) checked.value.clear();
  else { checked.value.clear(); sortedFiles.value.forEach(f => checked.value.add(f.id)); }
}

/**
 * 切换单个文件的选中状态
 *
 * @param id - 文件 ID
 */
function handleToggleFile(id: string): void {
  if (checked.value.has(id)) checked.value.delete(id); else checked.value.add(id);
}

/**
 * 格式化文件大小显示
 *
 * @param bytes - 字节数
 */
function formatSize(bytes: number): string {
  return formatFileSize(bytes);
}

/**
 * 格式化时间显示
 *
 * @param iso - ISO 时间字符串
 */
function formatTime(iso?: string | null): string {
  return formatDateTime(iso);
}

/**
 * 相对路径（跳过根节点名）
 *
 * @param f - 文件对象
 */
function relPathOf(f: DriveFile): string {
  // 根节点是展示节点，不属于挂载目录内的相对路径。
  const segs = browser.pathStack.slice(1).map(p => p.name);
  segs.push(f.name);
  return segs.join("/");
}

function syncStatusIcon(f: DriveFile): string {
  // 当前文件的批量状态缓存。
  const status = getFileStatus(f);
  if (status === "synced") return "local";
  if (status === "folder") return "folder";
  return "cloud";
}

function syncStatusText(f: DriveFile): string {
  // 当前文件的批量状态缓存。
  const status = getFileStatus(f);
  if (status === "synced") return SYNC_STATUS_SYNCED_LOCAL;
  if (status === "placeholder") return SYNC_STATUS_PLACEHOLDER;
  if (status === "folder") return SYNC_STATUS_FOLDER;
  return SYNC_STATUS_CLOUD_ONLY;
}

/**
 * 同步状态 CSS 类名
 *
 * @param f - 文件对象
 */
function syncStatusClass(f: DriveFile): string {
  // 当前文件的批量状态缓存。
  const status = getFileStatus(f);
  if (status === "synced") return "is-synced-local";
  if (status === "placeholder") return "is-placeholder";
  if (status === "folder") return "is-folder-status";
  return "is-cloud-only";
}

/**
 * 文件类型色块 tile 的配色类名（v2：folder 橙 / doc 靛 / image 粉 / video 紫 / file 灰）
 *
 * @param f - 文件对象
 */
function fileTileClass(f: DriveFile): string {
  if (driveApi.isFolder(f)) return "ftile--folder";
  // 服务端类别统一转小写映射视觉样式。
  const cat = (f.category ?? "").toLowerCase();
  switch (cat) {
    case "image": return "ftile--image";
    case "video": return "ftile--video";
    case "document": return "ftile--doc";
    default: return "ftile--file";
  }
}

/**
 * 双击文件行：文件夹→进入云端目录，文件→确保下载后用系统默认应用打开
 *
 * @param f - 文件对象
 */
async function handleDoubleClick(f: DriveFile): Promise<void> {
  if (driveApi.isFolder(f)) {
    await browser.enterFolder(f);
  } else {
    await handleOpenLocalItem(f);
  }
}

/**
 * 用系统默认应用打开本地项目。
 *
 * Linux 按需云盘直接把可见路径交给系统默认应用，由 FUSE 在应用首次 read 时透明下载。
 * 传统镜像模式下，普通文件仍先读取实时本地状态：已同步时立即打开，仅云端或占位状态
 * 则等待下载完成后再打开。文件夹不触发递归同步。
 *
 * @param f - 文件或文件夹对象
 */
async function handleOpenLocalItem(f: DriveFile): Promise<void> {
  await runOpenItem(async () => {
    closeMenu();
    if (!sync.mountConfigured) {
      showToast("请先在设置中配置同步目录", { variant: "warning" });
      return;
    }
    // 后端只接收挂载根内的相对路径，并负责最终路径安全校验。
    const relPath = relPathOf(f);
    try {
      if (!sync.usesVirtualDrive && !driveApi.isFolder(f)) {
        // 不依赖可能过期的列表缓存；打开前重新读取磁盘上的真实状态。
        const status = await commands.syncCheckFileLocalStatus(f.id);
        if (status !== "synced") {
          // 文件尚未落地时先完成下载；失败则不得继续打开空占位文件。
          const downloaded = await handleSyncFile(f, false);
          if (!downloaded) return;
        }
      }
      await commands.openLocalItem(relPath);
    } catch (e) {
      if (sync.usesVirtualDrive) await sync.refreshVirtualDriveStatus();
      showToast("打开失败：" + extractErrorMessage(e), { variant: "error" });
    }
  });
}

/**
 * 在系统文件管理器中打开文件所在目录。
 *
 * 此操作不下载文件；仅云端文件也可以直接查看其已经建立的本地镜像目录。
 *
 * @param f - 目标文件
 */
async function handleRevealLocalItem(f: DriveFile): Promise<void> {
  await runOpenItem(async () => {
    closeMenu();
    if (!sync.mountConfigured) {
      showToast("请先在设置中配置同步目录", { variant: "warning" });
      return;
    }
    try {
      await commands.revealLocalItem(relPathOf(f));
    } catch (e) {
      if (sync.usesVirtualDrive) await sync.refreshVirtualDriveStatus();
      showToast("打开所在文件夹失败：" + extractErrorMessage(e), { variant: "error" });
    }
  });
}

/**
 * 同步该目录/文件：文件夹→递归同步子树，文件→downloadOnDemand
 *
 * @param f - 文件对象
 */
async function handleSyncItem(f: DriveFile): Promise<void> {
  await runSyncItem(async () => {
    closeMenu();
    if (!fileOp.guard({ requireMount: true })) return;
    if (driveApi.isFolder(f)) {
      // 文件夹：递归同步子树（下载缺失 + 上传本地独有 + 建目录），进度弹窗
      await doSyncFolder(f);
    } else {
      // 文件：按需下载到镜像目录
      await handleSyncFile(f);
    }
  });
}

/**
 * 递归同步文件夹子树（在 runSyncItem 内调用，不再自裹防重复）。
 *
 * 后台异步执行：后端立即返回，不阻塞 UI。进度实时出现在传输队列（托盘图标 + 传输弹窗），
 * 用户可继续操作其他功能。完成无需 toast（传输队列本身显示完成态 + 后端会广播目录刷新）。
 *
 * @param f - 文件对象
 */
async function doSyncFolder(f: DriveFile): Promise<void> {
  // 后端递归同步所需的挂载相对路径。
  const rel = relPathOf(f);
  showToast(`已开始同步文件夹「${f.name}」，进度见传输队列`);
  // 后端命令只负责入队，等待调用结果不会阻塞实际后台同步。
  try {
    await commands.syncFolderRecursive(f.id, rel);
  } catch (e) {
    showToast("同步失败：" + extractErrorMessage(e), { variant: "error" });
  }
}

/**
 * 同步单个文件：下载到本地镜像目录
 *
 * @param f - 文件对象
 * @param notifySuccess - 下载成功后是否显示“已同步”提示；下载后立即打开时无需重复提示
 * @returns 是否成功完成下载
 */
async function handleSyncFile(f: DriveFile, notifySuccess = true): Promise<boolean> {
  if (driveApi.isFolder(f)) return false;
  if (!fileOp.guard({ requireMount: true })) return false;
  // 文件在挂载目录中的安全目标路径。
  const dest = `${mountDir.value}/${relPathOf(f)}`;
  downloading.value = { open: true, name: f.name };
  try {
    await commands.syncDownloadOnDemand(f.id, dest);
    if (notifySuccess) showToast(`已同步「${f.name}」`);
    // 下载完成后磁盘 xattr 已变（state=downloaded），重新拉批量状态刷新图标（云端→已同步）
    void refreshBatchStatus();
    return true;
  } catch (e) {
    showToast("同步失败：" + extractErrorMessage(e), { variant: "error" });
    return false;
  } finally {
    downloading.value.open = false;
  }
}

/**
 * 显示右键操作菜单
 *
 * @param e - 鼠标事件
 * @param f - 目标文件
 */
async function handleShowActionMenu(e: MouseEvent, f: DriveFile): Promise<void> {
  e.preventDefault();
  // 释放空间：文件夹只要挂载已配置即可（后端会递归枚举可释放文件）；
  // 单文件需 check_safe_free_up=="safe"（已下载、非占位、无活动传输）才显示。
  let canFreeUp = false;
  if (sync.mountConfigured) {
    if (driveApi.isFolder(f)) {
      canFreeUp = true;
    } else {
      try { canFreeUp = await commands.syncCheckSafeFreeUp(relPathOf(f), f.id) === "safe"; } catch {}
    }
  }
  contextMenu.value = { show: true, x: e.clientX, y: e.clientY, file: f, canFreeUp };
  nextTick(clampMenuToViewport);
}

/**
 * 关闭右键菜单
 */
function closeMenu(): void { contextMenu.value.show = false; }

/**
 * 菜单定位钳制：右/下溢出视口时翻转方向（向左/向上展开），保证完整可见。
 */
function clampMenuToViewport(): void {
  // 已渲染的菜单元素用于读取实际尺寸。
  const el = ctxMenuEl.value;
  if (!el) return;
  // 菜单与视口边缘的最小留白。
  const MARGIN = 8;
  // 菜单实际宽度。
  const w = el.offsetWidth;
  // 菜单实际高度。
  const h = el.offsetHeight;
  // 原始点击横坐标。
  const ox = contextMenu.value.x;
  // 原始点击纵坐标。
  const oy = contextMenu.value.y;
  // 钳制后的横坐标。
  let x = ox;
  // 钳制后的纵坐标。
  let y = oy;
  if (x + w > window.innerWidth - MARGIN) x = ox - w; // 右溢出 → 向左展开
  if (y + h > window.innerHeight - MARGIN) y = oy - h; // 下溢出 → 向上展开
  if (x < MARGIN) x = MARGIN;
  if (y < MARGIN) y = MARGIN;
  contextMenu.value = { ...contextMenu.value, x, y };
}

/**
 * 打开重命名对话框
 *
 * @param f - 要重命名的文件
 */
function handleRename(f: DriveFile): void {
  renameTarget.value = f; renameValue.value = f.name;
  showRenameDialog.value = true; closeMenu();
}

/**
 * 确认重命名
 */
async function handleConfirmRename(): Promise<void> {
  if (!renameTarget.value) return;
  if (!fileOp.guard()) return;
  // 去除用户无意输入的首尾空白。
  const newName = renameValue.value.trim();
  if (!newName || newName === renameTarget.value.name) { showRenameDialog.value = false; return; }
  showRenameDialog.value = false;
  // 关闭弹窗前保存目标，避免响应式状态后续被清空。
  const target = renameTarget.value;
  await fileOp.runAction(
    { errorPrefix: "重命名", successToast: "已重命名" },
    async () => { await commands.driveRenameFile(target.id, newName); },
  );
}

function handleShowProps(f: DriveFile): void {
  propsTarget.value = f; showPropsDialog.value = true; closeMenu();
}

/**
 * 删除文件（到回收站）
 *
 * @param f - 文件对象
 */
async function handleDelete(f: DriveFile): Promise<void> {
  closeMenu();
  if (!fileOp.guard()) return;
  // ★ 检查本地同步状态，决定删除确认文案
  let localStatus = "not_synced";
  if (sync.mountConfigured) {
    try { localStatus = await commands.syncCheckFileLocalStatus(f.id); } catch { /* ignore */ }
  }
  // 文件夹与文件使用不同的风险提示。
  const isFolder = driveApi.isFolder(f);
  // 最终确认文案。
  let content: string;
  if (isFolder) {
    content = `确定删除文件夹「${f.name}」吗？删除后进入回收站。`;
  } else if (localStatus === "synced") {
    content = `确定删除「${f.name}」吗？\n\n⚠️ 此文件已双端对齐到本地，删除后云端和本地文件将同时被移除。删除后进入回收站，可从回收站恢复。`;
  } else {
    content = `确定删除「${f.name}」吗？删除后进入回收站。`;
  }
  // 用户确认结果。
  const ok = await confirmDialog({
    title: "删除文件", titleIcon: "trash", danger: true, confirmText: "删除",
    content,
  });
  if (!ok) return;
  // 手动处理而非 runAction，以区分「真删除失败」与「文件已删但留痕失败」。
  try {
    await commands.driveDeleteFile(f.id, f.name);
    showToast("已删除");
  } catch (e) {
    // 错误信息
    const msg = extractErrorMessage(e);
    if (msg.startsWith(driveApi.DELETE_TRACE_ERROR_PREFIX)) {
      // 文件已删除，仅传输记录未写入
      showToast(`已删除「${f.name}」，但传输记录未写入`, { variant: "warning" });
    } else {
      showToast(`删除失败：${msg}`, { variant: "error" });
    }
  } finally {
    await browser.refresh();
  }
}

/**
 * 释放本地文件空间（保留云端占位）
 *
 * @param f - 文件对象
 */
async function handleFreeUpSpace(f: DriveFile): Promise<void> {
  closeMenu();
  if (!fileOp.guard()) return;
  // 文件夹递归枚举子树可释放文件；单文件只取自身（后端按 SYNCED 基线过滤）
  try {
    // 可释放候选清单（文件夹递归、单文件取自身）
    const items = await commands.syncListFreeableInFolder(relPathOf(f));
    if (items.length === 0) {
      showToast(driveApi.isFolder(f) ? "该目录下没有可释放的文件" : "该文件未同步到本地，无可释放项", { variant: "warning" });
      return;
    }
    freeUpPreviewItems.value = items;
    showFreeUpDialog.value = true;
  } catch (e) {
    showToast("查询可释放文件失败：" + extractErrorMessage(e), { variant: "error" });
  }
}

/**
 * 确认释放预览弹窗中的候选项：逐项释放，统计结果
 */
async function handleConfirmFreeUp(): Promise<void> {
  if (freeUpPreviewItems.value.length === 0) return;
  // 待释放候选项快照（确认期间预览不变）
  const items = [...freeUpPreviewItems.value];
  freeUpConfirmLoading.value = true;
  try {
    // 批量释放结果（成功/跳过计数与原因）
    const result = await commands.syncFreeUpBatch(items);
    // 跳过项附带前若干条原因，便于用户定位未释放文件。
    const skipDetail = result.skippedCount > 0 && result.errors.length > 0
      ? `\n跳过 ${result.skippedCount} 项：\n${result.errors.slice(0, 5).join("\n")}${result.errors.length > 5 ? `\n…等 ${result.errors.length} 条` : ""}`
      : (result.skippedCount > 0 ? `\n跳过 ${result.skippedCount} 项` : "");
    showToast(
      result.freedCount > 0
        ? `已释放 ${result.freedCount} 项（${formatFileSize(result.freedBytes)}）${skipDetail}`
        : `没有文件被释放（可能已被同步状态变更）${skipDetail}`,
      // 有跳过项时即使部分成功也用 warning，避免用户误认为全部完成
      { variant: result.freedCount > 0 && result.skippedCount === 0 ? "success" : "warning" },
    );
    showFreeUpDialog.value = false;
    freeUpPreviewItems.value = [];
    // 批量释放场景需清空选中
    if (checked.value.size > 0) checked.value.clear();
    await browser.refresh();
  } catch (e) {
    showToast("释放空间失败：" + extractErrorMessage(e), { variant: "error" });
  } finally {
    freeUpConfirmLoading.value = false;
  }
}

/**
 * 批量删除选中文件
 */
async function handleBulkDelete(): Promise<void> {
  await runBulkDelete(async () => {
    if (checked.value.size === 0) return;
    if (!fileOp.guard()) return;
    // ★ 检查选中项中是否有本地已同步的文件
    let syncedCount = 0;
    if (sync.mountConfigured) {
      for (const id of checked.value) {
        try {
          // 当前文件的本地同步状态。
          const status = await commands.syncCheckFileLocalStatus(id);
          if (status === "synced") syncedCount++;
        } catch { /* ignore */ }
      }
    }
    // 根据已同步项目数量补充双端删除风险。
    let content = `确定删除选中的 ${checked.value.size} 项吗？删除后进入回收站。`;
    if (syncedCount > 0) {
      content = `确定删除选中的 ${checked.value.size} 项吗？\n\n⚠️ 其中 ${syncedCount} 项已双端对齐到本地，删除后云端和本地文件将同时被移除。删除后进入回收站，可从回收站恢复。`;
    }
    // 用户确认结果。
    const ok = await confirmDialog({
      title: "批量删除", titleIcon: "trash", danger: true, confirmText: "删除",
      content,
    });
    if (!ok) return;
    // 批量循环：逐项删除。留痕失败（文件已删但记录未写入）算删除成功，单独统计；
    // 其他错误才是真失败。所有失败项收集原因并展示，不静默吞错。
    // 已删除文件数（含留痕失败）
    let deletedCount = 0;
    // 留痕失败项数（文件已删但传输记录未写入）
    let traceFailedCount = 0;
    // 真正删除失败的项（文件未删）
    const failedItems: string[] = [];
    // 留痕失败的项名（提示用户记录缺失）
    const traceFailedItems: string[] = [];
    await fileOp.runAction(
      { errorPrefix: "批量删除", refreshAfter: true, clearSelectionAfter: true },
      async () => {
        for (const f of sortedFiles.value) {
          if (!checked.value.has(f.id)) continue;
          try {
            await commands.driveDeleteFile(f.id, f.name);
            deletedCount++;
          } catch (e) {
            // 错误信息
            const msg = extractErrorMessage(e);
            if (msg.startsWith(driveApi.DELETE_TRACE_ERROR_PREFIX)) {
              // 留痕失败：文件已删，仅记录未写入
              deletedCount++;
              traceFailedCount++;
              traceFailedItems.push(f.name);
            } else {
              failedItems.push(`${f.name}：${msg}`);
            }
          }
        }
      },
    );
    // 失败详情（真失败 + 留痕失败分别提示）
    const failDetail = failedItems.length > 0
      ? `\n删除失败 ${failedItems.length} 项：\n${failedItems.slice(0, 5).join("\n")}${failedItems.length > 5 ? `\n…等 ${failedItems.length} 条` : ""}`
      : "";
    // 留痕失败详情（展示具体文件名，让用户知道哪些缺记录）
    const traceDetail = traceFailedCount > 0
      ? `\n${traceFailedCount} 项已删除但传输记录未写入：\n${traceFailedItems.slice(0, 5).join("\n")}${traceFailedItems.length > 5 ? `\n…等 ${traceFailedItems.length} 项` : ""}`
      : "";
    // 真删除失败或留痕失败时都用 warning（传输记录缺失也是需关注的状态）
    const hasIssue = failedItems.length > 0 || traceFailedCount > 0;
    showToast(
      `已删除 ${deletedCount} 项${failDetail}${traceDetail}`,
      { variant: hasIssue ? "warning" : "success" },
    );
  });
}

/**
 * 批量下载选中文件到本地
 */
async function handleBulkDownload(): Promise<void> {
  await runBulkDownload(async () => {
    if (checked.value.size === 0) return;
    if (!fileOp.guard({ requireMount: true })) return;
    // 本轮成功入队的文件数。
    let n = 0;
    await fileOp.runAction(
      { errorPrefix: "批量下载", refreshAfter: false, clearSelectionAfter: true },
      async () => {
        for (const f of sortedFiles.value) {
          if (!checked.value.has(f.id) || driveApi.isFolder(f)) continue;
          try { await commands.syncDownloadOnDemand(f.id, `${mountDir.value}/${relPathOf(f)}`); n++; } catch { /* 部分失败静默 */ }
        }
      },
    );
    showToast(`已下载 ${n} 项`);
  });
}

/**
 * 批量释放选中文件的本地空间
 */
async function handleBulkFreeUp(): Promise<void> {
  await runBulkFreeUp(async () => {
    if (checked.value.size === 0) return;
    if (!fileOp.guard()) return;
    // 逐个选中项枚举可释放候选项（文件取自身、目录递归子树），合并进同一预览弹窗。
    // 枚举失败收集原因：全部失败时报错；部分失败时预览弹窗提示清单可能不完整。
    // 合并后的全部可释放候选清单
    const all: FreeableItem[] = [];
    // 枚举失败项的原因（用于部分失败提示）
    const enumErrors: string[] = [];
    for (const f of sortedFiles.value) {
      if (!checked.value.has(f.id)) continue;
      try {
        // 当前选中项的可释放候选
        const items = await commands.syncListFreeableInFolder(relPathOf(f));
        all.push(...items);
      } catch (e) {
        enumErrors.push(`${f.name}：${extractErrorMessage(e)}`);
      }
    }
    if (all.length === 0) {
      // 无候选时优先解释枚举失败原因。
      const reason = enumErrors.length > 0
        ? `无可释放项，且 ${enumErrors.length} 项枚举失败：\n${enumErrors.slice(0, 3).join("\n")}`
        : "选中的项均未同步到本地，无可释放项";
      showToast(reason, { variant: "warning" });
      return;
    }
    if (enumErrors.length > 0) {
      showToast(`部分目录枚举失败（${enumErrors.length} 项），预览清单可能不完整`, { variant: "warning" });
    }
    freeUpPreviewItems.value = all;
    showFreeUpDialog.value = true;
  });
}

function handleSort(field: "name" | "size" | "modifiedTime"): void {
  if (sortField.value === field) sortAsc.value = !sortAsc.value;
  else { sortField.value = field; sortAsc.value = true; }
}
</script>

<template>
  <div class="file-list" :style="{ '--size-col-width': sizeWidth + 'px', '--time-col-width': timeWidth + 'px' }" @mousemove="onDrag" @mouseup="endDrag" @mouseleave="endDrag">
    <!-- 批量操作栏（v2：深色悬浮胶囊） -->
    <div v-if="checkedCount > 0" class="bulk-bar">
      <span class="bulk-bar__count">已选 {{ checkedCount }} 项</span>
      <button class="bulk-btn" :disabled="bulkDownloadLoading || bulkFreeUpLoading || bulkDeleteLoading || sync.isIndexing" @click="handleBulkDownload">
        <MateIcon name="download" :size="16" :spin="bulkDownloadLoading" /> 批量下载
      </button>
      <button class="bulk-btn" :disabled="bulkDownloadLoading || bulkFreeUpLoading || bulkDeleteLoading" @click="handleBulkFreeUp">
        <MateIcon name="cloud" :size="16" :spin="bulkFreeUpLoading" /> 释放空间
      </button>
      <button v-if="sync.mountConfigured" class="bulk-btn bulk-btn--danger" :disabled="bulkDownloadLoading || bulkFreeUpLoading || bulkDeleteLoading || sync.isIndexing" @click="handleBulkDelete">
        <MateIcon name="trash" :size="16" :spin="bulkDeleteLoading" /> 批量删除
      </button>
      <button class="bulk-close" title="取消选择" @click="checked.clear(); showCheckboxes = false">
        <MateIcon name="x" :size="14" />
      </button>
    </div>

    <!-- 空状态 -->
    <MateEmpty
      v-if="files.length === 0 && !browser.loading"
      icon="folder-open"
      title="此文件夹为空"
      description="上传或拖入文件即可同步到云端"
    />

    <!-- 加载态 -->
    <div v-if="browser.loading" class="file-loading"><MateCircularProgress :size="24" /></div>

    <!-- 拖拽导入高亮遮罩（pointer-events 关闭，不拦截拖放事件） -->
    <div v-if="dragImportOver" class="drop-overlay">
      <div class="drop-overlay__badge"><MateIcon name="upload" :size="28" /></div>
      <div class="drop-overlay__title">松开以上传到当前文件夹</div>
      <div class="drop-overlay__desc">文件将复制到同步目录，并自动上传到云端</div>
    </div>

    <template v-if="files.length > 0">
      <div class="file-header">
        <div class="file-header__checkbox">
          <MateCheckbox v-if="showCheckboxes" :model-value="headerCheck" tristate @update:model-value="handleToggleSelectAll" />
          <MateButton v-else variant="icon" icon="check" tooltip="多选" @click="showCheckboxes = true" />
        </div>
        <div class="file-header__name" @click="handleSort('name')">
          名称 <MateIcon v-if="sortField === 'name'" name="arrow" :size="12" :class="{ 'is-desc': !sortAsc }" />
        </div>
        <div class="file-header__size" @click="handleSort('size')">
          大小 <MateIcon v-if="sortField === 'size'" name="arrow" :size="12" :class="{ 'is-desc': !sortAsc }" />
          <div class="resize-handle" @mousedown="startDrag('size', $event)" />
        </div>
        <div class="file-header__time" @click="handleSort('modifiedTime')">
          修改时间 <MateIcon v-if="sortField === 'modifiedTime'" name="arrow" :size="12" :class="{ 'is-desc': !sortAsc }" />
          <div class="resize-handle" @mousedown="startDrag('time', $event)" />
        </div>
        <div class="file-header__status">状态</div>
        <div class="file-header__actions">操作</div>
      </div>

      <!-- 文件行 -->
      <div class="file-body">
        <div
          v-for="f in sortedFiles"
          :key="f.id"
          class="file-row"
          :class="{ 'is-selected': selectedId === f.id }"
          @click="selectedId = f.id"
          @dblclick="handleDoubleClick(f)"
          @contextmenu="handleShowActionMenu($event, f)"
        >
          <div class="file-col file-col--checkbox">
            <MateCheckbox v-if="showCheckboxes" :model-value="checked.has(f.id)" @update:model-value="handleToggleFile(f.id)" />
          </div>
          <div class="file-col file-col--name">
            <span class="ftile" :class="fileTileClass(f)">
              <img v-if="isThumbnailType(f) && thumbUrl(f)" :src="thumbUrl(f)" class="ftile__thumb" />
              <MateIcon v-else :name="driveApi.fileTypeIcon(f)" :size="18" />
            </span>
            <span class="file-name" :title="f.name">{{ f.name }}</span>
          </div>
          <div class="file-col file-col--size">
            {{ driveApi.isFolder(f) ? "—" : formatSize(f.size) }}
          </div>
          <div class="file-col file-col--time">
            {{ formatTime(f.edited_time) }}
          </div>
          <div class="file-col file-col--status" :title="syncStatusText(f)">
            <MateIcon :name="syncStatusIcon(f)" :size="16" :class="syncStatusClass(f)" />
          </div>
          <div class="file-col file-col--actions">
            <MateButton variant="icon" icon="list" tooltip="操作" @click.stop="handleShowActionMenu($event, f)" />
          </div>
        </div>
      </div>
    </template>

    <!-- 底部信息 -->
    <div class="file-footer">{{ files.length }} 项 · 已全部加载</div>

    <!-- 右键菜单（MateIcon 项） -->
    <Teleport to="body">
      <div v-if="contextMenu.show" class="ctx-capture" @click="closeMenu" @contextmenu.prevent="closeMenu" />
      <div v-if="contextMenu.show && contextMenu.file" ref="ctxMenuEl" class="ctx-menu menu-fade-in" :style="{ '--menu-x': contextMenu.x + 'px', '--menu-y': contextMenu.y + 'px' }">
        <button v-if="sync.mountConfigured" class="ctx-item" @click="handleOpenLocalItem(contextMenu.file!)">
          <MateIcon :name="driveApi.isFolder(contextMenu.file!) ? 'folder-open' : 'file'" :size="16" />
          {{ driveApi.isFolder(contextMenu.file!) ? "在系统文件管理器中打开" : "打开" }}
        </button>
        <button v-if="sync.mountConfigured && !driveApi.isFolder(contextMenu.file!)" class="ctx-item" @click="handleRevealLocalItem(contextMenu.file!)"><MateIcon name="folder-open" :size="16" /> 打开所在文件夹</button>
        <div v-if="sync.mountConfigured" class="ctx-sep" />
        <button v-if="sync.mountConfigured" class="ctx-item" :disabled="sync.isIndexing" @click="handleSyncItem(contextMenu.file!)"><MateIcon name="sync" :size="16" /> 执行双端对齐</button>
        <div v-if="sync.mountConfigured" class="ctx-sep" />
        <button v-if="contextMenu.canFreeUp" class="ctx-item" @click="handleFreeUpSpace(contextMenu.file!)"><MateIcon name="cloud" :size="16" /> 释放空间</button>
        <div v-if="contextMenu.canFreeUp" class="ctx-sep" />
        <button v-if="sync.mountConfigured" class="ctx-item" :disabled="sync.isIndexing" @click="handleRename(contextMenu.file!)"><MateIcon name="edit" :size="16" /> 重命名</button>
        <button class="ctx-item" @click="handleShowProps(contextMenu.file!)"><MateIcon name="info" :size="16" /> 属性</button>
        <div v-if="sync.mountConfigured" class="ctx-sep" />
        <button v-if="sync.mountConfigured" class="ctx-item ctx-item--danger" :disabled="sync.isIndexing" @click="handleDelete(contextMenu.file!)"><MateIcon name="trash" :size="16" /> 删除</button>
      </div>
    </Teleport>

    <!-- 重命名对话框 -->
    <MateDialog :open="showRenameDialog" title="重命名" @update:open="(v) => (showRenameDialog = v)">
      <MateTextField v-if="renameTarget" v-model="renameValue" autofocus placeholder="新名称" width="100%" @enter="handleConfirmRename" />
      <template #footer>
        <MateButton variant="text" @click="showRenameDialog = false">取消</MateButton>
        <MateButton variant="primary" icon="check" @click="handleConfirmRename">确定</MateButton>
      </template>
    </MateDialog>

    <!-- 属性对话框 -->
    <MateDialog :open="showPropsDialog && !!propsTarget" :title="propsTarget?.name ?? ''" @update:open="(v) => (showPropsDialog = v)">
      <div v-if="propsTarget" class="props-list">
        <div class="props-row"><span class="props-label">文件 ID</span><span class="props-value props-mono">{{ propsTarget.id }}</span></div>
        <div class="props-row"><span class="props-label">类型</span><span class="props-value">{{ driveApi.isFolder(propsTarget) ? "文件夹" : (propsTarget.mime_type ?? "文件") }}</span></div>
        <div class="props-row"><span class="props-label">大小</span><span class="props-value">{{ driveApi.isFolder(propsTarget) ? "—" : formatSize(propsTarget.size) }}</span></div>
        <div class="props-row"><span class="props-label">修改时间</span><span class="props-value">{{ formatTime(propsTarget.edited_time) }}</span></div>
        <div v-if="propsTarget.content_hash" class="props-row"><span class="props-label">SHA256</span><span class="props-value props-mono">{{ propsTarget.content_hash }}</span></div>
      </div>
      <template #footer>
        <MateButton variant="primary" @click="showPropsDialog = false">关闭</MateButton>
      </template>
    </MateDialog>

    <!-- 下载进度对话框 -->
    <MateDialog :open="downloading.open" title="下载中" :close-on-overlay="false" @update:open="(v) => (downloading.open = v)">
      <div class="dl-pane">
        <MateCircularProgress :size="20" />
        <span class="dl-name">{{ downloading.name }}</span>
      </div>
    </MateDialog>

    <!-- 释放空间预览对话框 -->
    <MateDialog :open="showFreeUpDialog" title="释放空间" title-icon="cloud" danger :close-on-overlay="!freeUpConfirmLoading" @update:open="(v) => (showFreeUpDialog = v)">
      <div class="freeup-pane">
        <p class="freeup-summary">
          共 {{ freeUpPreviewItems.length }} 项，可释放
          <strong>{{ formatFileSize(freeUpTotalBytes) }}</strong>
        </p>
        <div class="freeup-list">
          <div v-for="it in freeUpPreviewItems" :key="it.fileId" class="freeup-row">
            <MateIcon name="file" :size="14" />
            <span class="freeup-row__name" :title="it.name">{{ it.name }}</span>
            <span class="freeup-row__size">{{ formatFileSize(it.size) }}</span>
          </div>
        </div>
      </div>
      <template #footer>
        <MateButton variant="text" :disabled="freeUpConfirmLoading" @click="showFreeUpDialog = false">取消</MateButton>
        <MateButton variant="primary" icon="cloud" :loading="freeUpConfirmLoading" @click="handleConfirmFreeUp">确认释放</MateButton>
      </template>
    </MateDialog>
  </div>
</template>

<style scoped>
.file-list { flex: 1; display: flex; flex-direction: column; overflow: hidden; position: relative; padding: 0 12px; }

/* 批量操作栏（v2：深色悬浮胶囊） */
.bulk-bar {
  height: 44px; display: flex; align-items: center; gap: 10px;
  margin: 10px 0 0; padding: 0 8px 0 16px;
  background: rgba(28, 28, 30, 0.94); border-radius: var(--radius-lg);
  color: #fff; box-shadow: var(--sh-lg); flex-shrink: 0;
}
.bulk-bar__count { font-size: var(--font-body-sm); font-weight: var(--fw-semibold); margin-right: auto; }
.bulk-btn {
  height: 32px; display: inline-flex; align-items: center; gap: 6px;
  padding: 0 12px; border: none; border-radius: var(--radius-md);
  background: transparent; color: rgba(255, 255, 255, 0.85);
  font-size: var(--font-body-sm); font-weight: var(--fw-medium);
  cursor: pointer; white-space: nowrap;
}
.bulk-btn:hover:not(:disabled) { background: rgba(255, 255, 255, 0.12); }
.bulk-btn:disabled { opacity: 0.45; cursor: not-allowed; }
.bulk-btn--danger { color: #fda4af; }
.bulk-btn--danger:hover:not(:disabled) { background: rgba(244, 63, 94, 0.18); }
.bulk-close {
  width: 28px; height: 28px; border: none; border-radius: 50%;
  background: transparent; color: rgba(255, 255, 255, 0.7);
  display: inline-flex; align-items: center; justify-content: center;
  cursor: pointer; flex-shrink: 0;
}
.bulk-close:hover { background: rgba(255, 255, 255, 0.12); color: #fff; }

/* 表头（v2：小号大写感标签） */
.file-header {
  height: var(--file-header-height); display: flex; align-items: center;
  border-bottom: 1px solid var(--line);
  font-size: 11.5px; font-weight: var(--fw-semibold); letter-spacing: 0.3px;
  color: var(--ink-400); flex-shrink: 0; padding: 0 12px;
}
.file-header__checkbox { width: 40px; display: flex; align-items: center; flex-shrink: 0; }
.file-header__name { flex: 1; cursor: pointer; user-select: none; display: flex; align-items: center; gap: var(--space-xs); }
.file-header__name:hover, .file-header__size:hover, .file-header__time:hover { color: var(--brand-500); }
.file-header__size, .file-header__time { flex-shrink: 0; cursor: pointer; user-select: none; position: relative; display: flex; align-items: center; gap: var(--space-xs); }
.file-header__size { width: var(--size-col-width, 110px); }
.file-header__time { width: var(--time-col-width, 160px); }
.resize-handle { position: absolute; right: 0; width: 6px; height: 100%; cursor: col-resize; }
.file-header__status { width: 72px; flex-shrink: 0; text-align: center; }
.file-header__actions { width: 44px; flex-shrink: 0; text-align: center; }
.is-desc { transform: rotate(90deg); }

/* 文件行（v2：56px 卡片化行） */
.file-body { flex: 1; overflow-y: auto; }
.file-row {
  height: var(--file-row-height); display: flex; align-items: center;
  padding: 0 12px; border-bottom: 1px solid var(--line);
  border-radius: var(--radius-md);
  transition: background-color 0.1s; cursor: default;
}
.file-row:hover { background-color: var(--bg-hover); }
.file-row.is-selected { background-color: var(--brand-50); }
.file-col--checkbox { width: 40px; flex-shrink: 0; display: flex; align-items: center; }
.file-col--name { flex: 1; min-width: 0; display: flex; align-items: center; gap: 12px; }
.file-col--size, .file-col--time { flex-shrink: 0; font-size: 12.5px; color: var(--ink-400); font-variant-numeric: tabular-nums; }
.file-col--size { width: var(--size-col-width, 110px); }
.file-col--time { width: var(--time-col-width, 160px); }
.file-col--status { width: 72px; display: flex; align-items: center; justify-content: center; color: var(--ink-300); flex-shrink: 0; }
.file-col--status :deep(.is-cloud-only) { color: var(--ink-300); }
.file-col--status :deep(.is-synced-local) { color: var(--ok); }
.file-col--status :deep(.is-placeholder) { color: var(--ink-600); }
.file-col--status :deep(.is-folder-status) { color: var(--brand-500); }
.file-col--actions { width: 44px; display: flex; align-items: center; justify-content: center; flex-shrink: 0; }
.file-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--font-body); color: var(--ink-900); }

/* 文件类型色块 tile（v2：32px 圆角 6px，柔和底色 + 彩色图标） */
.ftile {
  width: 32px; height: 32px; border-radius: 6px; flex-shrink: 0;
  display: flex; align-items: center; justify-content: center;
}
.ftile--folder { background: #fff4de; color: #f0a63c; }
.ftile--doc { background: #eef2ff; color: #6366f1; }
.ftile--image { background: #fde7f3; color: #ec4899; }
.ftile--video { background: #f3e8ff; color: #8b5cf6; }
.ftile--file { background: var(--bg-fill); color: var(--ink-400); }
.ftile__thumb { width: 100%; height: 100%; border-radius: 6px; object-fit: cover; }

/* 加载 / 空态 */
.file-loading { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; background: rgba(255, 255, 255, 0.6); }

/* 拖拽导入高亮遮罩 */
.drop-overlay {
  position: absolute; inset: 0; z-index: 40;
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: var(--space-sm); border-radius: var(--radius-lg);
  background: var(--brand-50); box-shadow: inset 0 0 0 2px var(--brand-200);
  pointer-events: none;
}
.drop-overlay__badge {
  width: 56px; height: 56px; display: flex; align-items: center; justify-content: center;
  border-radius: 50%; background: var(--grad-brand-soft); color: var(--brand-500);
}
.drop-overlay__title { font-size: 15px; font-weight: var(--fw-semibold); color: var(--text-primary); }
.drop-overlay__desc { font-size: var(--font-body-sm); color: var(--text-secondary); }

/* 底部 */
.file-footer {
  height: 36px; display: flex; align-items: center; justify-content: center;
  font-size: var(--font-caption); color: var(--ink-300); flex-shrink: 0;
}

/* 右键菜单（v2：毛玻璃弹层） */
.ctx-capture { position: fixed; inset: 0; z-index: 1500; }
.ctx-menu {
  position: fixed; z-index: 1501; min-width: 200px;
  background: rgba(255, 255, 255, 0.96); backdrop-filter: blur(16px);
  border-radius: var(--radius-lg);
  box-shadow: var(--sh-pop), 0 0 0 0.5px rgba(0, 0, 0, 0.05);
  padding: 6px; left: var(--menu-x, 0); top: var(--menu-y, 0);
  display: flex; flex-direction: column; gap: 1px;
}
.ctx-item {
  display: flex; align-items: center; gap: 10px; width: 100%; height: 36px;
  padding: 0 12px; font-size: var(--font-body); text-align: left;
  background: none; border: none; border-radius: var(--radius-md);
  cursor: pointer; color: var(--ink-900); white-space: nowrap;
}
.ctx-item:hover { background: var(--bg-fill); }
.ctx-item--danger { color: var(--err); }
.ctx-item--danger:hover { background: var(--err-bg); }
.ctx-item:disabled { color: var(--ink-300); cursor: not-allowed; }
.ctx-item:disabled:hover { background: none; }
.ctx-sep { height: 1px; background: var(--line); margin: 5px 10px; }

/* 属性列表（v2 props-row） */
.props-list { display: flex; flex-direction: column; }
.props-row { display: flex; align-items: baseline; padding: 8px 0; border-bottom: 1px solid var(--line); }
.props-row:last-child { border-bottom: none; }
.props-label { width: 88px; flex-shrink: 0; font-size: var(--font-body-sm); color: var(--ink-400); }
.props-value { flex: 1; font-size: var(--font-body-sm); color: var(--ink-900); word-break: break-all; }
.props-mono { font-family: var(--font-mono); font-size: var(--font-caption); }

/* 下载进度 */
.dl-pane { display: flex; align-items: center; gap: var(--space-md); }
.dl-name { font-size: var(--font-body); color: var(--ink-900); }

/* 释放空间预览 */
.freeup-pane { display: flex; flex-direction: column; gap: var(--space-sm); }
.freeup-summary { font-size: var(--font-body-sm); color: var(--ink-600); margin: 0; }
.freeup-summary strong { color: var(--ink-900); }
.freeup-list { max-height: 280px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
.freeup-row { display: flex; align-items: center; gap: var(--space-sm); padding: 4px 0; font-size: var(--font-body-sm); color: var(--ink-900); }
.freeup-row__name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.freeup-row__size { flex-shrink: 0; color: var(--ink-400); font-variant-numeric: tabular-nums; }
</style>
