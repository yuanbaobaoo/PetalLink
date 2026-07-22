# PetalLink 架构优化分析报告

> 产出形式：**仅分析，不含代码改动**。每项给出问题、建议方案、收益、风险与预估工作量。
> 分析时点：v2 视觉重构完成后（2026-07）。行数以 `wc -l` 实测为准。
> 编码约束：拆分不得夹带业务逻辑变更（coding-rules §1.2 / §3.3）；生产 Rust 文件上限 1000 行。

---

## 一、前端（app/）

### 1.1 `FileListView.vue`（1026 行）—— 全仓最大文件，建议拆分

**问题**：单文件承载了六类职责——列宽拖拽、多选状态、右键菜单定位钳制、行渲染（ftile/状态图标/缩略图）、四个对话框（重命名/属性/释放空间/下载进度）、批量操作。控制流需跳读，审阅成本高；v2 重画后行数进一步增长（938 → 1026）。

**建议方案**（按职责边界拆，行为不变）：

| 拆出物 | 内容 | 预估行数 |
|--------|------|----------|
| `composables/useColumnResize.ts` | `sizeWidth`/`timeWidth`/`startDrag`/`onDrag`/`endDrag` 纯逻辑 | ~60 |
| `views/main/FileListRow.vue` | 单行渲染：ftile、状态图标、缩略图、复选框 | ~120 |
| `views/main/FileContextMenu.vue` | 右键菜单 Teleport + 定位钳制 + 菜单项 | ~120 |
| `views/main/FileListDialogs.vue` | 重命名/属性/释放空间/下载进度四个 MateDialog | ~200 |

主文件保留：数据编排（排序、多选、批量操作、状态查询），预计回落到 ~500 行。

**收益**：主文件职责单一（列表编排）；行组件可独立审阅；列宽逻辑可复用。
**风险**：低。纯视图拆分，props/emits 边界清晰；需注意 `fileStatuses`、`thumbUrls` 缓存的传递路径不要复制状态。
**工作量**：约 0.5 天（含 vitest 补一条 FileListRow 渲染用例）。

### 1.2 `SettingsPage.vue`（528 行）—— 按面板拆子组件

**问题**：六个设置面板（同步目录/传输/高级/账号/日志/关于）全部堆在一个文件，v2 重画引入 `settings-panel` 白卡结构后模板进一步膨胀。

**建议方案**：抽 `views/settings/panels/` 目录，每面板一个组件（`SyncDirPanel.vue` / `TransferPanel.vue` / `AdvancedPanel.vue` / `AccountPanel.vue` / `AboutPanel.vue`），主文件保留导航、配置加载与保存编排。配置 state 目前集中在主文件（`concurrency` 等），建议保持现状由主文件 `v-model` 下发，**不要为了拆分引入新的状态管理**。

**收益**：面板独立演进（后续加设置项只动一个文件）；主文件回到 ~200 行。
**风险**：低-中。`handleSave` 依赖多个面板的 ref，拆分后需保持配置读取路径单一（建议仍由主文件持有配置 state）。
**工作量**：约 0.5 天。

### 1.3 删除死依赖 `vue-router`

**问题**：`app/package.json` 声明 `vue-router ^4.4.0`，全仓零引用——页面切换是 `App.vue` 的 `currentPage` ref + v-if。

**建议方案**：`npm uninstall vue-router`，同时删除 `package.json`/`package-lock.json` 条目。
**收益**：减少依赖体积与安全审计面；消除"到底用没用路由"的认知噪音。
**风险**：无（零引用）。若未来引入多窗口/深链接再装回。
**工作量**：10 分钟。

### 1.4 `MainPage.vue` 搜索结果区可抽组件（可选，优先级低）

**问题**：搜索结果渲染（`search-results`/`search-row`）约 30 行模板内嵌在 MainPage。
**建议**：抽 `views/main/SearchResults.vue`（props：keyword/results/isSearching，emit：enter-folder）。
**收益**：小；MainPage 更聚焦工具栏编排。**可不做**——当前 MainPage 仅 142 行，未超阈值。

### 1.5 设计令牌旧别名渐进清理（长期）

**背景**：v2 重画采用"保留旧语义别名映射到新值"的过渡方案（`--color-brand` → `var(--brand-500)` 等），存量组件未全部改写。
**建议**：后续触碰某个组件时顺手将其样式迁移到 v2 原生命名（`--brand-500`/`--ink-900`/`--bg-fill`/`--line`）， aliases 全部下线后从 tokens.css 删除。**不建议一次性全局替换**（diff 噪音大、无行为收益）。
**风险**：低，但全量替换会产生大面积无意义 diff，违背可审阅性原则。

---

## 二、Rust 后端（src/）

### 2.1 `src/commands/drive.rs`（894 行）—— 查询/写操作混杂，建议拆分

**问题**：全仓最接近 1000 行上限的文件，且职责混杂：
- 查询类薄命令（`drive_list`/`drive_get_file`/`drive_create_folder` 之外多为 5-15 行包装）；
- 写操作重业务逻辑——`drive_delete_file`（约 350 行，含本地清理）、`drive_rename_file`、`drive_move_file`（含 xattr 与 `path_recovery` 协作）、`drive_upload_file`。

**建议方案**（对齐 coding-rules §3.3 按领域职责拆分）：

```
src/commands/drive.rs        —— 门面：模块声明 + 共享类型 re-export（薄）
src/commands/drive_query.rs  —— 查询类命令（list/get/search/thumbnail/about）
src/commands/drive_write.rs  —— 写操作命令（delete/rename/move/upload/create_folder）
```

`lib.rs` 的 `tauri::generate_handler!` 注册路径改为子模块路径；函数体、错误映射、日志语义逐字搬移。

**收益**：写操作的高风险逻辑（删除/移动涉及本地文件与 xattr）获得独立审阅边界；远离 1000 行红线。
**风险**：低。纯搬移；注意 `pub(crate)` 可见性不要扩大，拆分前后用 diff 核对仅文件归属变化。
**工作量**：约 0.5 天（含 `cargo test --all-targets` 全量回归）。

### 2.2 xattr 操作封装泄漏 —— 建议收敛到 MountManager

**问题**：xattr 的领域归属在 `mount/manager.rs`（定义 `XATTR_FILE_ID`/`XATTR_STATE` 等键），但 commands 层 4 个文件直接调用 `xattr::get/set/remove`（`drive.rs`、`free_up.rs`、`folder_sync.rs`、`sync_status.rs`），绕过 MountManager 封装。xattr 键名与值语义（占位符状态机）实际由 mount 模块定义，commands 层直接读写意味着**状态机契约分散在两处**。

**建议方案**：在 `MountManager` 上收敛为一组语义化方法，例如：

```rust
// 读取占位符状态（不存在/占位/已下载）
pub fn placeholder_state(&self, path: &Path) -> PlaceholderState;
// 标记/清除占位符（写 xattr + 不变量校验）
pub fn mark_placeholder(&self, path: &Path, file_id: &str) -> Result<()>;
pub fn clear_placeholder(&self, path: &Path) -> Result<()>;
```

commands 层改为调用这些方法，不再直接 `use xattr`。
**收益**：xattr 键名/状态值单点定义，未来改键名或加字段只动一处；commands 层代码更贴近业务语义。
**风险**：中。涉及 4 个命令文件的调用点改写，需逐个核对错误映射（xattr IO 错误的 AppError 映射文案不能变）；建议配合 `tests/` 中现有 free_up / folder_sync 相关用例回归。
**工作量**：约 1 天。

### 2.3 大文件观察名单（暂不拆，持续增长时预拆）

| 文件 | 行数 | 预拆方向（达到 ~900 行时） |
|------|------|---------------------------|
| `sync/engine/reconciliation.rs` | 759 | 按记录收敛阶段拆 read/merge/write |
| `mount/manager.rs` | 759 | xattr 操作随 §2.2 收敛后自然瘦身 |
| `drive/download_api.rs` | 723 | 按 request/response/stream 拆 |
| `sync/executor/actions.rs` | 720 | 按动作类型（上传/下载/删除）拆 |
| `sync/executor/transfer_operations.rs` | 717 | 与 actions 同步评估 |

这些文件当前单一职责清晰，**现在拆是无意义抽象**（coding-rules §1.2 禁止），列入观察即可。

---

## 三、正面结论（保持不变）

1. **sync 引擎三层拆分规范**：`engine/`（编排）→ `executor/`（执行）→ `task_runner/`（持久任务），每层内部按 admission/execution/settlement 等阶段再拆，文件 300-600 行为主，是全仓结构标杆。
2. **Mate 组件库粒度合理**：27 个组件分类清晰（图标/品牌/按钮/输入/选择/进度/反馈/菜单/展示/基础设施），`useDialog`/`useToast` 模块级服务 + Host 组件模式避免了全局状态污染，v2 重画验证了复用价值（视觉升级只动组件库 + 页面样式，未改业务逻辑）。
3. **contract 测试机制有效**：`app/tests/*.contract.test.ts` 直接读 Rust 源码锁定前后端常量（`TransferState` discriminant、命令名），零 mock 成本，建议在新增协议字段时沿用。
4. **commands 层薄 + 运行时单例集中**：`commands.rs` 仅承载全局单例与引擎所有权协议，49 个 command 按 9 个领域文件分布，结构健康。

---

## 四、实施优先级建议

| 优先级 | 事项 | 理由 |
|--------|------|------|
| P0 | §1.3 删除 vue-router | 零风险、即时收益 |
| P1 | §2.1 drive.rs 拆分 | 唯一逼近 1000 行红线 |
| P1 | §1.1 FileListView 拆分 | 前端唯一超 1000 行 |
| P2 | §2.2 xattr 收敛 | 契约单点化，配合回归测试 |
| P2 | §1.2 SettingsPage 拆分 | 面板独立演进 |
| P3 | §1.4 / §1.5 / §2.3 | 可选或观察项 |

> 本报告仅作分析，任何实施需单独排期并遵循 coding-rules 的拆分纪律（diff 核对、行为不变、同步更新 README/docs）。
