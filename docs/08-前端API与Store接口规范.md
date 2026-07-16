# 08 · 前端 API 与 Store 接口规范

> 原项目前端为 Vue3 + Pinia。Kuikly 重构后，这些「API 调用」变成 **ViewModel → 业务层的方法**，「Pinia store」变成 **ViewModel/StateFlow**。本文档记录前端完整类型接口规范与状态管理逻辑，供重构对照。
>
> 本文基于原项目 `app/` 全部前端源码（flat 结构，约 8000 行）的逐行核对。

---

## 一、前端整体架构

### 1.1 目录结构（flat，无 src/ 层级）

```
app/
├── main.ts                  # 入口：createApp + Pinia + 4 事件监听
├── App.vue                  # 根组件路由（currentPage ref 手动切换）
├── api/                     # Tauri invoke 封装（8 个模块）
│   ├── tauri.ts             #   invoke/on 统一封装 + AppError 类型
│   ├── auth.ts drive.ts sync.ts transfer.ts config.ts platform.ts updater.ts logs.ts
├── stores/                  # Pinia store（5 个，全部 setup 风格）
│   ├── auth.ts sync.ts fileBrowser.ts transfer.ts updater.ts
├── views/
│   ├── LoginPage.vue
│   ├── main/                # 主界面（MainPage + 6 子组件）
│   └── settings/            # 设置页 + 日志页
├── components/
│   ├── IconSprite.vue       # 全局 SVG sprite（32 个图标）
│   ├── UpdateDialog.vue     # 更新对话框（独立状态机）
│   └── mate/                # Mate 组件库（27 个 + index.ts）
├── composables/             # useAsyncAction / useFileOperation
├── utils/                   # error.ts / format.ts / fs.ts
├── styles/                  # reset.css / tokens.css / animations.css
└── tests/                   # vitest 契约/状态测试
```

### 1.2 依赖清单

| 依赖 | 版本 | 用途 |
|---|---|---|
| vue | ^3.5.0 | 框架（Composition API + `<script setup>`） |
| pinia | ^2.2.0 | 状态管理（setup store 风格） |
| typescript | ^5.6.0 | 严格类型 |
| vite | ^5.4.0 | 构建（目标 safari13） |
| vitest + @vue/test-utils + jsdom | ^2.1 / ^2.4 / ^25 | 测试 |
| @tauri-apps/api + plugins | ^2.0.0 | IPC（invoke/listen/window） |
| vue-router | ^4.4.0 | **已装但未用**（路由靠 App.vue currentPage ref） |

### 1.3 main.ts 全局事件监听（4 个 Tauri 事件）

挂载后才注册事件（延迟 import stores 避免 Pinia 未就绪）：

| 事件 | 处理逻辑 |
|---|---|
| `sync_state` | `useSyncStore().applyState(payload)`。**只承载状态**，队列变化由独立 transfer_update |
| `folder_content_changed` | `useFileBrowserStore().refresh()` + `sync.sidebarRefresh++`（计数器触发 watch，布尔无法重复触发） |
| `transfer_update` | `useTransferStore().loadAll()` |
| `upload_failed` | `showToast("上传失败：...", {variant:"error"})`。**5 秒去重**：相同 msg 5s 内只弹一次（防重试风暴刷屏） |

所有 `.catch(()=>{})` 吞掉注册失败。

### 1.4 App.vue 根组件路由

- `showSplash = status==="initial" && loading`
- 路由：`initial+loading→Splash`；`loggedIn+settings→SettingsPage`；`loggedIn+logs→LogViewerPage`；`loggedIn+main→MainPage`；其他→LoginPage
- 顶层挂载全局宿主：`<IconSprite/>`、`<MateDialogHost/>`、`<MateToastHost/>`、`<UpdateDialog/>`
- **onMounted 流程**：`auth.restore()` → 若已登录 `sync.init()` → 注册 `navigate_settings` 事件 → 更新检查三件套：
  - 启动 3s 后 `updater.silentCheck()`（强制不节流）
  - `setInterval(periodicCheck, CHECK_INTERVAL_MS=1h)`
  - `getCurrentWindow().onFocusChanged` → 聚焦时 `checkOnFocus()`（10 分钟节流）
- onUnmounted 清理所有 timer 与 unlistenFocus

---

## 二、五个 Pinia Store 完整规格

> 全部为 setup store（`defineStore("name", () => {...})`），返回 ref/computed/action。

### 2.1 auth store — 登录态状态机

**State**：`status`（`initial|authorizing|loggedIn|loggedOut|error`）、`loading`、`errorMessage`、`secretConfigured`、`callbackPort`（默认 9999）、`userInfo: UserInfo|null`

**状态机流转**：
```
initial --restore()--> loggedIn | loggedOut | error
loggedOut --login()--> authorizing --> loggedIn | error | loggedOut(用户取消)
loggedIn --logout()--> loggedOut
error --dismissError()--> loggedOut
```

**Actions**：
- `restore()`：checkSecret → restore → 设 status → 若登录再 getUserInfo（失败仅 warn）
- `login(): Promise<boolean>`：**防重复**（loading 中 return false）→ authorizing → `authApi.login(port)` → 成功后①getUserInfo ②**补调 `useSyncStore().init()`**（因 onMounted 在未登录时已跑过、未调 sync.init）。用户取消（msg 含"用户取消授权"）静默回 loggedOut
- `cancelLogin()`、`dismissError()`、`logout()`

### 2.2 sync store — 同步全局状态（最复杂，21 个字段）

**State**：`revision`（单调版本号）、`total/completed/uploading/downloading/waitingNetwork/failed/transferFailed`、`failedItems: FailedItem[]`、`conflict`、`editing`、`isRunning`、`isIndexing`、`indexingScannedFolders`、`indexingDiscoveredItems`、`syncPhase`、`lastSyncTime`、`contentChanged`、`sidebarRefresh`（刷新计数器）、`mountConfigured`、`mountDir`、`setupPhase`（`loading|needsSetup|needsFirstSync|active`）

**Computed**：`progress`（total=0 返回 1.0，否则 completed/total）、`hasActiveTransfer`（uploading+downloading+waitingNetwork>0）

**关键 action `applyState(value): boolean`**：
1. `isSyncGlobalState(value)` 校验失败 → return false（拒缺字段/revision=0 默认对象）
2. **`s.revision < revision.value → return false`**（旧 revision 直接拒绝）
3. 记录 `isNewRevision = s.revision > revision.value`
4. 逐字段赋值全部计数器 + failedItems（展开拷贝）
5. `content_changed` 处理：true 则置位，**仅 isNewRevision 时 sidebarRefresh++**（同一 revision 重复投递只允许幂等赋值，不能重复触发刷新）

**`init()`**：loadConfig 判断 setupPhase；配置就绪时**主动拉一次 `getSyncState()` 并 applyState**（避免错过配置完成前已发出的 is_indexing 事件——BFS 可能先于 init 启动）

### 2.3 fileBrowser store — 路径栈 + 文件列表

**State**：`pathStack: FolderLocation[]`（初始 `[ROOT={id:"",name:"我的云盘"}]`）、`files: DriveFile[]`、`loading`、`errorMessage`

**Computed**：`current`（pathStack 末尾）

**Actions**：`loadCurrent()`（`driveApi.listFiles(current.id||undefined)`，folders-first 排序）、`enterFolder(f)`（仅文件夹，push+loadCurrent）、`jumpTo(i)`（slice 到第 i 级）、`goUp()`、`refresh()`

### 2.4 transfer store — 传输队列（两重乱序保护）

**State**：`tasks: TransferTask[]` + 两个**模块级闭包变量**（非响应式）：`nextLoadRequest`、`lastAppliedLoadRequest`

**Computed（14 个派生）**：`uploads`、`downloads`（含 DOWNLOAD_UPDATE）、`running`、`pending`、`waitingNetwork`、`backingOff`、`verifyingRemote`、`restartRequired`、`completed`、`failed`、`canceled`、`processing`（running+verifyingRemote）、`waiting`（pending+waitingNetwork+backingOff+restartRequired）、`active`（processing+waiting）、`hasActiveTasks`

**关键 action `loadAll(): Promise<boolean>`（两重乱序保护）**：
1. `requestId = ++nextLoadRequest`
2. `listAllTransfers()`
3. **第一重**：`if (requestId < lastAppliedLoadRequest) return false`（旧请求丢弃）
4. **第二重（同 task 旧 revision 回写保护）**：建 `currentRevisions = Map(tasks.id → state_revision)`，若 loaded 中任一 task 的 `state_revision < currentRevision` → return false
5. 通过则 `tasks.value = loaded`、`lastAppliedLoadRequest = requestId`
6. catch 返回 false（IPC 失败≠空队列，**保留最后成功快照**）

### 2.5 updater store — 更新全流程状态机

**常量**：`CHECK_INTERVAL_MS = 1h`、`FOCUS_THROTTLE_MS = 10min`

**State**：`phase`（`idle|checking|available|upToDate|downloading|downloaded|waitingTransfer|ready|error`）、`updateInfo`、`downloadProgress`、`lastCheckTime`、`dialogOpen`

**Actions**：`doCheck()`（静默失败，更新 lastCheckTime）、`silentCheck()`（强制不节流）、`throttledCheck(ms)`、`periodicCheck()`、`checkOnFocus()`、`manualCheck(): Promise<boolean>`（有更新自动弹窗）、`downloadAndInstall()`（回调式进度：Started 设 total、Progress 累加百分比封顶 99、Finished 设 100）、**`waitForTransfers(): Promise<boolean>`**（轮询 hasActiveTransfers，**最多等 5 分钟每 2s 一次**；完成则 phase=ready+弹窗；超时回 downloaded+弹窗）、`dismissUpdate()`

---

## 三、API 层

### 3.1 tauri.ts — invoke/on 封装 + AppError

```typescript
interface AppError {
  kind: "Auth" | "Token" | "DriveApi" | "Config" | "QuotaExceeded" | "Generic";
  message: string;            // 始终为字符串，用户可读中文
  code?: string | null;       // 子错误码（Auth/Token/DriveApi 有）
  status_code?: number | null; // DriveApi 特有
  error_code?: string | null;  // DriveApi 特有
}
```
- `invoke<T>(command, args?)`：try tauriInvoke，catch 时若对象含 `kind` 直接抛，否则包装成 `{kind:"Generic", message}`
- `on<T>(event, handler)`：`listen(event, e => handler(e.payload))`（**解包 payload**）

### 3.2 各模块导出函数

- **auth.ts**：`checkSecret()`、`restore(): AuthState`、`login(port): TokenPair`、`cancelLogin()`、`logout()`、`getUserInfo(): UserInfo`、`isLoggedIn()`。+ `primaryLabel/secondaryLabel/initial`（CJK 安全取首字符）
- **drive.ts**：`listFiles(parentId?)`（folders-first）、`searchFiles`、`createFolder`、`deleteFile(id, name?)`、`renameFile`、`getThumbnail(fileId): string|null`（bytes→base64 data URL）、`getAbout()`、`downloadFile`、`uploadFile`。+ 常量 **`DELETE_TRACE_ERROR_PREFIX = "TRACE_FAILED:"`**（前后端约定）+ `isFolder(f)`（大小写不敏感）、`fileTypeIcon(f)`
- **sync.ts**：`manualRefresh()`、`checkSafeFreeUp(relPath, fileId)`、`checkFileLocalStatus(fileId)`、`getBatchFileStatus(fileIds[])`、`freeUpSpace(...)`、`listFreeableInFolder(folderRelPath)`、`freeUpBatch(items)`、`downloadOnDemand(fileId, destPath)`、`syncFolderRecursive(folderId, relPath)`、`retryFailed()`、`getSyncState()`。+ `isSyncGlobalState()` 强校验器
- **transfer.ts**：常量 `TRANSFER_DIR`（UPLOAD=0/DOWNLOAD=1/DELETE=2/DOWNLOAD_UPDATE=3）、`TRANSFER_STATE`（9 态 0-8）、`TRANSFER_OPERATION`（8 种 0-7）、`TRANSFER_ERROR_KIND`（12 种 0-11）。函数：`listAllTransfers()`、`clearCompleted/clearFailed/clearFinished`、`retryTransfer(taskId)`。+ `canRetryTransferTask(task)`
- **config.ts**：`loadConfig()`、`saveConfig(config)`、`exportConfigJson()`、`importConfigJson(jsonStr)`、`clearCache()`
- **platform.ts**：`openInFinder(path)`、`launchAtLoginIsEnabled()`、`launchAtLoginSetEnabled(enabled)`、`getAppVersion()`
- **updater.ts**：`checkForUpdate(): UpdateInfo|null`（静默失败 null）、`downloadAndInstall(onProgress?)`（Started/Progress/Finished 三阶段）、`hasActiveTransfers()`
- **logs.ts**：`listLogs()`、`exportLogs(path)`、`clearLogs()`

### 3.3 utils

- `error.ts`：`extractErrorMessage(e)`（优先 .message，回退 String）
- `format.ts`：`formatFileSize(bytes)`（0 返回"—"；B/KB/MB/GB/TB 自适应，B 无小数其余 1 位）、`formatDateTime`（YYYY-MM-DD HH:mm）
- `fs.ts`：`isEmptyDir(dir)`（readDir 后过滤隐藏 + SKIP_PATTERNS=[.DS_Store,.tmp,~$*,.Trash]，同步目录选择校验）

---

## 四、页面结构

### 4.1 LoginPage.vue
渐变背景（135deg #EBF1FF→#F5F5F5→#FFF）+ 3 个装饰圆（品牌色 opacity 0.04-0.06）+ 居中卡片（max-width 480）。卡片：MateAppLogo + 标题 + 品牌分隔线 + secretWarning/error banner + 主按钮。授权中：spinner + 取消按钮。`canLogin = secretConfigured && !loading`

### 4.2 MainPage.vue（布局壳）
flex 布局：左侧 `<Sidebar/>`（220px）+ 右侧 main-content。AppBar(56px)：搜索框（MateSearchField，max-width 280）+ 同步索引/传输队列/Finder 按钮 + 设置图标按钮。info-area：SyncSetupBanner / SyncStatusBar / 错误 banner。Breadcrumb + 文件区。

### 4.3 FileListView.vue（最大文件，938 行）— 核心

**列定义（6 列）**：checkbox(40px) / name(flex:1) / size(var(--size-col-width,100px)) / time(var(--time-col-width,150px)) / status(60px) / actions(40px)

**列宽拖拽**：size/time 列有 `.resize-handle`（6px col-resize），mousedown 记录 dragStartX/dragStartW，mousemove 算 `newW = clamp(64, 400, dragStartW + dx)`

**排序**：`sortField: name|size|modifiedTime`、`sortAsc`。sortedFiles **文件夹优先**，同类型内按字段（name 用 localeCompare，size 数值差，time 字符串）

**多选**：`checked: Set<string>`、`showCheckboxes`（默认隐藏，点表头 check 图标显示）、`selectedId`（单选聚焦高亮）。表头 tri-state：`headerCheck = false(0选)|true(全选)|null(部分)`

**右键菜单**（`@contextmenu`）：`handleShowActionMenu` 异步先查 `canFreeUp`（文件夹只要挂载配置即 true；单文件需 `checkSafeFreeUp==="safe"`），设 contextMenu，nextTick 调 `clampMenuToViewport`（MARGIN=8：右溢出向左、下溢出向上）

**同步状态展示**：`refreshBatchStatus()` 批量查 `getBatchFileStatus(ids)`。状态映射：synced→local/绿色；placeholder→cloud/灰；folder→folder/品牌色；not_synced→cloud/placeholder色

**文件操作**（通过 `useFileOperation` composable）：guard（索引中/未配置目录 toast 拦截）→ 执行 action → 成功 toast + 可选 refresh，失败 toast
- 双击：文件夹→enterFolder，文件→handleSyncFile（downloadOnDemand）
- 右键"执行双端对齐"：文件夹→syncFolderRecursive；文件→handleSyncFile
- 删除：先查 checkFileLocalStatus，synced 时确认文案含⚠️双端删除警告。留痕失败（`TRACE_FAILED:` 前缀）单独提示
- 释放空间：`listFreeableInFolder` 枚举 → 预览对话框 → `freeUpBatch`
- 批量操作栏（checkedCount>0）：批量下载/释放空间/批量删除

### 4.4 TransferPopover.vue（420×560）
Header + 统计栏（处理中/等待中/已完成/历史失败 + 清除菜单）+ 单列表。**无 Tab 切换**——全部任务单列表展示，靠 `stateMeta` Record 映射每状态的 icon/label/color/spin。重试按钮仅 `canRetryTransferTask` 为 true 时显示，`retryingId` 防抖

### 4.5 SyncStatusBar.vue
`statusText` 优先按 `syncPhase` 精确匹配 8 阶段，无 sync cycle 时回退到持久化队列状态判断。**关键：不能把等待/退避/核验误显示为完成**。onMounted 主动 loadAll 避免误报

### 4.6 Sidebar.vue + SidebarTreeNode.vue
递归目录树。**并发安全 + 路径联动**：
- `loadToken` 序号：并发 loadChildren 时只有最新请求结果能写入 children
- `pendingRelink`：加载期间路径变化标记
- `relinkRetryCount`（上限 MAX_RELINK_RETRIES=2）
- watch `sync.sidebarRefresh`（计数器）刷新已展开节点；watch `browser.current.id` 联动展开

### 4.7 SettingsPage.vue（6 Tab）
左导航 200px（MateNavItem ×6）+ 右设置区。Tab：syncDir / transfer / advanced / account / logs / about

---

## 五、Mate 组件库（27 个）

`components/mate/index.ts` 统一导出。分 8 组：

### 图标/品牌（3）
- **MateIcon**：`{name, size=16, spin=false}`。SVG `<use href="#i-${name}">`
- **MateAppLogo**：`{size=26, text="PetalLink", container=false}`
- **MateLogoWithText**：`{height=32}`

### 按钮（1）
- **MateButton**：`{variant: primary|text|icon|icon-text, danger, disabled, loading, fullWidth, tooltip, icon, badge=0, height=0}`。emit `click`。loading 时 primary 显示 spinner。icon 变体 32×32。badge>99 显示"99+"。**hover 仅背景色过渡，无 ripple**

### 输入（4）
- **MateTextField**：`{modelValue, placeholder, autofocus, disabled, prefixIcon, width, type, fontSize, fill, error, maxlength}`。emit `update:modelValue`、`enter`
- **MateNumberField**：`{modelValue, min=0, max=999999, width=120, suffix, disabled}`
- **MateStepper**：`{modelValue, min=0, max=999999, step=1}`。32px 高
- **MateSearchField**：`{modelValue, placeholder, width, maxWidth}`。emit `submit`

### 选择（4）
- **MateSwitch**：`{modelValue, disabled}`。40×22
- **MateCheckbox**：`{modelValue: boolean|null, tristate, disabled, size}`。支持 tri-state（null=部分）
- **MateRadio**：`{value, modelValue, disabled, size}`
- **MateRadioGroup**：`{modelValue}`

### 进度（2）
- **MateLinearProgress**：`{value: number|null, height=4, color}`
- **MateCircularProgress**：`{size=24, strokeWidth=2.5, color, value=null}`

### 反馈（5，核心）
- **MateInfoBanner**：`{variant: info|warning|error|success, title, closable}`。slot + `#action`
- **MateDialog**：`{open, title, titleIcon, danger, closeOnOverlay=true, width=420}`。emit `update:open`、`close`。Teleport to body
- **MateDialogHost**：绑定 `useDialog` 模块级 `dialogState`
- **MateToastHost**：绑定 `useToast` 的 `toasts`（底部居中 bottom:48px）
- **useDialog**（非组件）：模块级 reactive。`openDialog(opts)`、`confirmDialog(opts): Promise<boolean>`（**Promise + resolver 实现命令式 await**）、`closeDialog(value=false)`
- **useToast**（非组件）：`toasts` reactive 数组。`showToast(message, {variant, duration=2000})`。**单条语义**：splice 清空旧的再 push 新的，2s 自动 dismiss

### 菜单（1）
- **MatePopupMenu**：`{items: PopupItem[], menuWidth=168, disabled}`。emit `select`。`inheritAttrs:false`。`openMenu` on pointerdown：右/左/下边界检查（预估 menuH=200）。全屏捕获层。Teleport to body

> **注**：FileListView 的右键菜单**没用** MatePopupMenu，而是自己实现（需异步查 canFreeUp 后动态决定菜单项）

### 展示（6）
- **MateEmpty**：`{icon, title, description}`
- **MateTag**：`{label, theme, size, icon}`
- **MateNavItem**：`{label, icon, active, indent=0, height=32}`
- **MateSectionHeader**：`{text, icon}`
- **MateStatChip**：`{icon, count, label}`
- **MateSpinningIcon**：`{name, size, color}`

### 基础设施（4）
- **MateHover**：`{cursor: pointer|default}`
- **MateVerticalSeparator**：`{height=20}`
- **MateBottomDivider**：`{background, color}`
- **MateScaffold**：`{flush=false}`。flex column 全屏

---

## 六、组合式函数

| 函数 | 用途 |
|---|---|
| `useAsyncAction` | 异步操作封装（loading/error 状态） |
| `useFileOperation` | 文件操作（guard 索引中/未配置目录 toast 拦截 → 执行 action → 成功 toast + 可选 refresh/clearSelection，失败 toast） |

---

## 七、关键重构提示（给 Kuikly）

1. **事件驱动架构**：UI 几乎所有状态更新靠 4 个 Tauri 事件，而非主动轮询。后端推送权威快照，前端 applyState 做旧 revision 拒绝 + 幂等赋值
2. **两重乱序保护**（transfer.loadAll）：请求序号 + per-task state_revision 比对，必须完整复刻否则会有状态回退 bug
3. **模块级状态 + Promise resolver**（useDialog/useToast）：命令式 `await confirmDialog()` 是核心交互模式，Kuikly 需等价机制（如回调/Channel）
4. **sidebarRefresh 计数器**：因 Vue watch 对布尔无法重复触发，用递增数字触发刷新；Kuikly 若用 Flow/State 需注意等价问题
5. **SidebarTreeNode 路径联动 + 并发安全**：loadToken + pendingRelink + relinkRetryCount 三件套是难点
6. **DELETE_TRACE_ERROR_PREFIX**：前后端硬编码约定字符串，需同步（有 contract test）
7. **路径**：flat 结构（无 src/），`@` 别名指向 `app/` 根
