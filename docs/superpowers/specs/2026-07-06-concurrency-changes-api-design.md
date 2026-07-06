# 并发模型增强 + 华为 changes API 接入设计

> 版本：1.0 · 2026-07-06
> 状态：已确认（待写实施计划）
> 关联：自动云端→本地同步（定时全量刷新，2026-07-02 已实现）

---

## 1. 背景与现状

用户提出 4 点并发增强诉求：
1. 索引进行中时，自动刷新应跳过（不重叠）
2. 自动刷新由独立线程完成
3. 多线程目录扫描（根据目录结构自动启动多任务）
4. 主动查询华为文件变化接口（单独线程）

**经调研确认的现状：**

| 诉求 | 现状 | 结论 |
|---|---|---|
| ① 跳过重叠 | `manual_syncing` 锁防并发 BFS，但无显式 `is_indexing` 检查 | 需补显式检查 |
| ② 独立线程 | `start_cloud_refresh_timer` 已是独立 `tokio::spawn` task | **已满足**，补日志可见性 |
| ③ 多线程目录扫描 | **云端 BFS 已 8 并发**（`join_all`，`INDEXING_CONCURRENCY=8`）；本地扫描串行 | 云端无需改；本地扫描串行但非本次痛点 → **不做** |
| ④ changes API | **完全未实现**（仅 `drive/mod.rs` TODO） | 新建模块，分两阶段 |

**关键澄清：**
- 本项目"线程" = tokio async task（全项目唯一 OS thread 是关闭 flush）。所有"独立线程"诉求映射为 `tokio::spawn` 专用任务，与现有惯例一致。
- 运行时为 Tauri 默认的 tokio 多线程池（worker = CPU 核心数）。
- 用户原以为云端 BFS"一个目录一个目录挨个扫"——**实际已是 8 并发**，③无需做。

---

## 2. 总体方案：三阶段

| 阶段 | 内容 | 风险 | 价值 | 依赖 |
|---|---|---|---|---|
| **阶段一** | ①②：自动刷新跳过索引 + 独立 task 日志可见性 | 低（约 10 行改动） | 修掉索引重叠卡死隐患，立即可见 | 无 |
| **阶段二** | ④-A：验证华为 changes 接口（独立 binary） | 低（只读探查） | 消除最大不确定性，为阶段三提供事实依据 | 无 |
| **阶段三** | ④-B：基于验证结果接入 changes API | 中（新模块） | 增量同步，省流量、提速 | 阶段二结果 |

阶段一与阶段二**可并行**（一个改主代码、一个独立 binary 探查）。阶段三依赖阶段二的验证报告。

---

## 3. 阶段一设计：自动刷新跳过索引 + task 可见性

### 3.1 显式检查 `is_indexing` 跳过

改造 `run_auto_cloud_refresh`（`src/sync/engine.rs:1208`），在开头加显式检查：

```rust
async fn run_auto_cloud_refresh(self: &Arc<Self>) {
    // ①索引中（含手动刷新/启动 BFS/其他自动刷新的 BFS 阶段）→ 跳过本次，等下次定时
    if self.is_indexing() {
        tracing::info!("自动云端刷新: 索引进行中，跳过本次");
        return;
    }
    // 现有 manual_syncing 锁检查（防并发 BFS）保留
    {
        let mut guard = self.manual_syncing.lock();
        if *guard {
            tracing::info!("自动云端刷新: 手动同步进行中，跳过本次");
            return;
        }
        *guard = true;
    }
    let result = self.run_auto_cloud_refresh_impl().await;
    *self.manual_syncing.lock() = false;
    if let Err(e) = result {
        tracing::warn!(error = %e, "自动云端刷新失败（忽略，下次定时重试）");
    }
}
```

**为什么 `is_indexing` 比 `manual_syncing` 更宽：** `is_indexing` 在任何 BFS（启动期 / 手动 / 自动）期间为 true，自动刷新看到它 true 就跳过——连"启动期首次 BFS 还没结束"也能正确跳过，而 `manual_syncing` 只防"手动刷新进行中"。

### 3.2 独立 task 可见性

现状 `start_cloud_refresh_timer`（`engine.rs:517`）已是独立 `tokio::spawn` task，**②已满足**。补强：在跳过 / 执行时都加 `tracing` 日志，使日志里可见"自动刷新 task 在独立运行"。

### 3.3 改动范围

仅 `src/sync/engine.rs` 的 `run_auto_cloud_refresh`：加 `is_indexing` 检查 + 跳过日志（约 10 行）。无 schema、无配置、无前端改动。

### 3.4 验证

- `cargo build --lib` + `cargo test --lib`
- 手工：启动期 BFS 进行中观察日志是否出现"索引进行中，跳过本次"

---

## 4. 阶段二设计：验证华为 changes 接口

### 4.1 目标

用真实 token 调 `/drive/v1/changes`，摸清接口行为，为阶段三设计提供事实依据。

### 4.2 验证方式

新建独立 dev binary `src/bin/changes_probe.rs`（参考现有 `src/bin/upload_tester.rs` 模式：`#[tokio::main]` + 从 env 读 token）。该 binary：
1. 从 `.env` 或环境变量读 access token（复用现有 token 获取逻辑）
2. 调 `GET /drive/v1/changes`（带不同参数组合）
3. 打印完整响应供分析

### 4.3 必须摸清的 5 个关键点

1. **初始 cursor 怎么拿？** GDrive 有 `changes/getStartPageToken`，华为有没有对应？还是首次直接调 `/changes` 不带 cursor？
2. **响应结构：** `changes`/`items` 数组？每个 change 的字段（含 file 元数据 + 变更类型）？`nextCursor`/`newStartCursor` 分页字段？
3. **变更类型：** 如何区分 文件新增/修改/删除？（GDrive 用 `removed: true`，华为呢？）
4. **cursor 持久性：** cursor 能跨会话/重启复用吗？
5. **最终一致性：** DELETE 后 changes 是否也会短暂返回旧数据（像 list 那样）？

### 4.4 为什么独立 binary 而非集成测试

接口未验证，wiremock 测不了真实行为；独立 binary 能快速迭代探查，不影响主代码。验证完后可保留为调试工具或删除。

### 4.5 产出

一份验证报告（追加到本文档第 6 节），确认或否定上述 5 点，作为阶段三的设计输入。

---

## 5. 阶段三设计：接入 changes API（框架）

> **注意：** 本节具体字段映射依赖阶段二验证结果。此处先定架构框架与接入点，具体字段（cursor 名、change 结构、removed 标志位）等验证后补。

### 5.1 新模块 `src/drive/changes_api.rs`

仿 `about_api.rs`（用 `DriveClient::get`，最简风格），在 `src/drive/mod.rs` 注册：

```rust
pub struct ChangesApi { client: Arc<DriveClient> }

impl ChangesApi {
    pub fn new(client: Arc<DriveClient>) -> Self { ... }
    /// 拉取增量变更（cursor=None 首次拉取；自动分页至 next_cursor 为空）
    pub async fn list_changes(&self, cursor: Option<&str>) -> AppResult<ChangeListResult> { ... }
}

pub struct ChangeListResult {
    pub changes: Vec<Change>,
    pub next_cursor: Option<String>,  // 持久化用；None 表示已追平
}

pub struct Change {
    pub kind: ChangeKind,   // Added / Modified / Removed（具体判定等阶段二）
    pub file: DriveFile,     // 复用现有模型
}

pub enum ChangeKind { Added, Modified, Removed }
```

自动分页仿 `files_api::list_all`（pageSize + cursor 循环，MAX_PAGES 兜底）。

### 5.2 cursor 持久化

cursor 存到 app 数据目录（`support_dir()/changes_cursor.txt`，仿 `cache_paths.rs` 风格）：
- 启动时加载 → 有则走增量，无则走全量
- 每次增量成功后更新
- 失效（接口报错）→ 删除文件，下次回退全量重建

### 5.3 接入 `run_auto_cloud_refresh_impl`（改造现有自动刷新）

```
run_auto_cloud_refresh_impl:
  广播 is_indexing=true
  有持久化 cursor 且有效?
    ├ 是 → 增量：list_changes(cursor) → merge 变更进内存 cloud_tree → 更新 cursor
    └ 否/失效 → 回退全量 BFS（现有 refresh_cloud_tree 逻辑）
  广播 is_indexing=false
  跑 run_sync_cycle("auto-cloud-refresh")
```

增量路径不全量 BFS，大幅省流量、提速；失败 / cursor 失效自动回退全量，保证正确性。

### 5.4 独立 task

增量轮询**复用现有 `start_cloud_refresh_timer` task**，不另起 task——它的职责就是"定时拉云端变更"，增量只是更高效的实现。满足用户"单独线程执行"。

### 5.5 ChangeKind → planner 动作

增量结果 merge 进 cloud_tree 后，planner 的现有 3-way diff 自然处理（cloud_tree 里新增/修改/删除的条目会被 diff 出对应动作）。**无需改 planner。**

### 5.6 风险兜底

- cursor 失效 / changes 接口异常 → 回退全量 BFS + 清 cursor 重建
- changes 返回的 DriveFile 字段不全 → 复用现有 `DriveFile::from_json` 的容错（多 key 探测、String 容忍）

---

## 6. 阶段二验证报告（已完成真机探查）

> 通过 `changes_probe` binary + 真实账号 OAuth 授权探查确认（2026-07-06）。

- [x] **初始 cursor 获取方式**：`GET /drive/v1/changes/getStartCursor`
  - 响应：`{"category":"drive#startCursor","startCursor":"311296"}`
  - **关键**：华为的 `/changes` 接口强制要求 cursor，无 cursor 直接 400 "Cursor can't be null"（21004001 LACK_OF_PARAM）。初始 cursor 必须先调 getStartCursor 获取，**不能**用 `list_changes(None)`（GDrive 风格）。
- [x] **响应结构（数组名、字段名）**：
  - 数组名：`changes` ✓（与 GDrive 一致）
  - 分页游标字段：**`newStartCursor`**（**非** GDrive 的 `nextCursor`）
  - 顶层 category：`drive#changeList`
  - 空变更响应示例：`{"category":"drive#changeList","changes":[],"newStartCursor":"311296"}`
- [x] **变更类型判定（removed 标志）**：已校准。华为用 **`changeType`** 字段区分，**非** GDrive 的 `removed` 布尔：
  - 删除（移入回收站）：`changeType == "trashDone"`（真机确认）
  - 增/改：`changeType` 为其他值（如 `update`/`create`，非 trashDone 即按 Modified 处理）
  - 关键差异：删除事件 `deleted` 恒为 `false`（华为用 changeType 而非 deleted）；且删除事件**仍带完整 file**（`file.recycled == true`），fileId 在顶层也有。
  - 代码判定：`changeType == "trashDone"` 为主，`file.recycled == true` 兜底。
- [x] **cursor 持久性**：**会过期**。`cursor=1` → 410 Gone "Cursor has expired"（errorCode `21084100`，reason `CURSOR_EXPIRED`）。代码已处理：过期的 cursor 调用会返回 Err → 引擎自动回退全量 BFS + 清 cursor 重建基线。
- [x] **最终一致性表现**：cursor 过期机制即是一致性保证（与 files/list 的最终一致性不同，changes 靠 cursor 时效）。

### 已校准的代码改动（基于本报告）
- `changes_api.rs`：新增 `get_start_cursor()` 方法；`from_json` 游标字段改为 `newStartCursor` 优先
- `engine.rs`：`try_init_changes_cursor_abs` 改用 `get_start_cursor()`（原 `list_changes(None)` 会 400）
- cursor 过期（410）由现有 `handle_error_response` → `drive_from_status` → Err 自动捕获，回退全量

---

## 7. 不做的事项（明确边界）

- **③本地扫描并行化**：云端 BFS 已 8 并发，本地扫描串行但非本次痛点（且 watcher 增量扫描只扫变更目录，快）。如未来本地全量扫描成瓶颈再单独处理。
- **OS 线程**：本项目惯例是 tokio async task，不引入 `std::thread`（除已有的关闭 flush）。所有"独立线程"= `tokio::spawn`。
- **改 planner / executor**：增量变更经 cloud_tree merge 后走现有 3-way diff，无需改决策逻辑。
- **改配置 / 前端**：阶段一不改配置；阶段三的 cursor 持久化对用户透明，无 UI 改动。

---

## 8. 验证与回滚

- 每阶段独立 `cargo build --lib` + `cargo test --lib` + 必要时 `npm run type-check`
- 阶段一约 10 行，回滚成本极低
- 阶段二独立 binary，删除即回滚
- 阶段三新模块 + 改造一处自动刷新，回滚 = 删 changes_api + 还原 run_auto_cloud_refresh_impl 到全量 BFS
