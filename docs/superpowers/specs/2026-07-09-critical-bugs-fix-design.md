# 严重 BUG 修复方案设计

> 日期：2026-07-09
> 版本：v1.0
> 状态：待审阅
> 影响版本：v1.0.7 及以前

## 0. 背景

本文档针对 PetalLink 经运行时日志与代码双重定位的 4 个严重 BUG，给出修复方案。每个 BUG 的根因已在前期分析中用实锤证据锁定，此处仅记录结论与修复点。

| BUG | 现象 | 根因结论 | 优先级 |
|---|---|---|---|
| BUG 2 | 大文件上传失败无提示、无法重试 | 自动上传失败无 toast + 传输队列无重试入口 + 失败进度归零 + 重试无断点续传 | 高 |
| BUG 3 | 断网恢复后点菜单栏崩溃闪退 | `panic="abort"` 放大器 + muda 0.19.3 `icon.rs:34` `write_header().unwrap()` 在 ZeroWidth 时 panic | 高 |
| BUG 4 | 重启后删本地未上传文件 / 副本反复删除重现 | BFS 子树永久失败仍写 `complete=true` + 删除分支无启动守卫 + 删本地无二次校验 | **致命** |

> BUG 1（日志导出）：用户反馈"导出文件只有当天内容"。经用 Rust 直接运行 `logs_export` 等价逻辑实测，当前能正确读入 logs 目录全部 12 个文件（跨度 14 天）并拼接为 4.2MB。代码逻辑（`commands.rs:1698` read_dir 全部文件）与 v1.0.7 二进制一致，无法从代码层复现。**根因待定**，但用户诉求明确——无论根因如何，导出必须保证全部日志。本次纳入为**增强项 REQ-2**，加固导出健壮性 + 可观测性。

> BUG 5（莫名出现云端副本）：经查证，这些"云端副本"是华为官方客户端在 2025 年生成的历史冲突副本，原本就存在于云端，PetalLink 只是如实镜像。其"删除又重现"是 BUG 4 的次生症状，随 BUG 4 修复一并解决。

---

## 0.1 新增需求（本轮补充）

除上述 BUG 修复外，用户追加两项需求：

| 需求 | 现象 | 根因 |
|---|---|---|
| **REQ-1** | 睡眠/断网时不应做任何同步/检测操作，只做网络连通性检测，等网络恢复再继续 | 当前**完全没有网络状态检测和睡眠监听**；`auto-cloud-refresh` 定时器（engine.rs:561-585）每 60s 无脑触发，网络断了也照常发起 BFS → 这是 BUG 3/4 的**上游触发源**（凌晨断网连续 BFS 失败写出残缺 cloud_tree） |
| **REQ-2** | 日志导出应支持导出全部日志 | 见上 BUG 1 说明 |

---

## 1. BUG 4 — 数据丢失（纵深防御三道全修）

### 1.1 根因（运行时实锤）

```
断网 → BFS 子目录连续失败 3 次（实锤：7/08 00:38-01:05 连续 10 次 "files=0 complete=true"）
  → cloud_tree.rs:166-176 仅记日志，不 return Err、不标记 incomplete
  → cloud_tree.rs:204 BFS 结束后无条件 persist_cloud_tree(complete=true)
  → 残缺但非空（含 3 个根目录节点）的缓存完美绕过 load_persisted_cloud_tree 的完整性校验
崩溃退出（BUG 3）
  → 重启 load_or_refresh_cloud_tree 加载残缺缓存到内存
  → run_sync_cycle("startup-resume")
  → planner 误判"本地有/云端无(files=0)/DB 有真实 id"
  → planner.rs:202-263 生成 DeleteFromLocal（此分支无 is_startup_resume 守卫）
  → executor.rs:871 do_delete_from_local 无"已上传云端"复核
  → manager.rs:282 对 size>0 真实文件无条件 remove_file → 数据丢失
  → apply_results + purge_stale_db_records 清掉 DB 痕迹
```

运行时铁证：`2026-07-06T23:44:51 actions=3496` 一次性误删 3496 个本地文件（含副本），触发源 `auto-cloud-refresh`。

### 1.2 设计：三道纵深防御

按用户决策"纵深防御三道全修"，任一道失效另两道兜底。

#### 第一道（根因）：BFS 完整性诚实化

**位置**：`src/sync/cloud_tree.rs`

**当前问题**：
- `refresh_cloud_tree`（L51-207）在 BFS 循环中（L166-176）文件夹永久失败时仅 `tracing::error!`，不传播错误。
- 循环结束后（L204）无条件 `persist_cloud_tree(complete=true)`。

**修复**：
1. 在 `refresh_cloud_tree` 内部维护一个 `permanent_failures: usize` 计数器，当某文件夹重试耗尽（`node.retries >= 2`）时累加。
2. BFS 结束后的持久化改为**条件写**：
   - `permanent_failures == 0` → 正常写 `complete=true`（保持现状）。
   - `permanent_failures > 0` → **不写 `complete=true` 缓存**，改为写 `complete=false` 哨兵（让下次启动强制重跑全量），并 `tracing::warn!` 记录失败子树数量。内存中的 cloud_tree 照常返回供本轮使用，但**不把这份残缺数据当作可信持久基线**。
3. 新增返回值或字段标识本轮是否"部分成功"（`partial: bool`），供 engine 决策（见第三道）。

**为什么这样**：保持"运行中"的 cloud_tree 可用性（避免单次网络抖动就让整个同步停滞），但**绝不把残缺结果固化为可信基线**。下次启动必然重跑全量 BFS，从源头消除"加载残缺缓存"路径。

**完整性校验增强**（`load_persisted_cloud_tree`，L240-264）：除 `complete=true && 非空` 外，增加**文件数合理性启发式**——若新加载的缓存文件数相比上次成功的内存计数（持久化一个 `last_known_file_count` 元字段）骤降超过阈值（如 < 50%），视为可疑，拒绝加载、强制重跑。这是对抗"恰好非空但严重残缺"的额外保险。

#### 第二道：启动恢复期删除守卫

**位置**：`src/sync/planner.rs`

**当前问题**：`local_exists && !cloud_exists && db_exists`（L202-263）整块**完全没有 `is_startup_resume` 检查**，而同文件的文件夹删除（L86）和会话内 DeleteFromCloud（L302）都有。对比测试（L707 注释）甚至明确标注"会触发 DeleteFromLocal 的危险条件"。

**修复**：在 L202 进入该分支块时，增加启动恢复期保护：

```rust
// === 本地有 + 云端无 ===
if local_exists && !cloud_exists {
    if db_exists {
        // ★ 启动恢复期：cloud_tree 可能不可信（BFS 部分失败/缓存残缺）。
        // 对"DB 有真实 fileId（非 pending:）且本地未改"的文件，绝不直接删除，
        // 改为 Skip 并记录"待复核"，等下一次 BFS 成功后重新判定。
        // 仅 pending: 占位项（上传待重试）和本地已改（BackupBeforeCloudDelete）不受影响。
        if snap.is_startup_resume
            && !db.unwrap().file_id.starts_with(PENDING_FILE_ID_PREFIX)
            && !is_local_changed(local.unwrap(), db.unwrap())
        {
            return Some(SyncAction {
                action_type: SyncActionType::Skip,
                relative_path: Some(rel_path.to_string()),
                file_id: db.unwrap().file_id.clone().into(),
                ...
                reason: Some("启动恢复期 cloud_tree 不可信，跳过删除待复核".to_string()),
            });
        }
        // ... 原 pending / 文件夹 / BackupBeforeCloudDelete / DeleteFromLocal 分支不变
    }
}
```

**为什么这样**：
- 仅在 `is_startup_resume=true` 时启用，**不影响会话内**（用户实时操作）的删除——会话内 cloud_tree 是内存实时版，可信。
- `pending:` 占位项不受保护（它本就该重传，不删除）。
- 本地已改的文件不受保护（走 BackupBeforeCloudDelete，本就不删内容）。
- 只拦截"本地未改 + DB 有真实 id"这一最危险的误删场景。

**后续恢复**：被 Skip 的文件在下一轮（BFS 成功、cloud_tree 完整）会重新走 planner，届时 `cloud_exists` 若为真则正常 skip，若为真删则正常删除——即"延迟到 cloud_tree 可信时再决策"。

#### 第三道：executor 删除前云端复核

**位置**：`src/sync/executor.rs`（`do_delete_from_local`，L871-909）+ `src/sync/engine.rs`（新增 `validate_delete_from_local`，对标 L1749 的 `validate_delete_from_cloud`）

**当前问题**：`engine.rs:677` 有 `validate_delete_from_cloud`（对删云端做 stat 二次校验），但**删本地（数据丢失更严重、不可恢复的方向）反而没有任何二次校验**。

**修复**：新增 `validate_delete_from_local`，在 executor 执行删除前对**有真实 fileId 的非占位文件**做一次云端 API 复核：

```rust
// engine.rs（run_sync_cycle_inner 中，validate_delete_from_cloud 之后）
self.validate_delete_from_local(&mut actions).await;
```

```rust
// engine.rs 新增
async fn validate_delete_from_local(&self, actions: &mut [SyncAction]) {
    for action in actions.iter_mut() {
        if action.action_type != SyncActionType::DeleteFromLocal { continue; }
        let Some(file_id) = &action.file_id else { continue; };
        if file_id.starts_with(PENDING_FILE_ID_PREFIX) { continue; } // 占位项不复核

        // 对真实 fileId 做一次 GET /drive/v1/files/{id}
        match self.api.get_file(file_id).await {
            Ok(_) => {
                // 云端确实存在 → planner 的 cloud_exists=false 是误判（cloud_tree 残缺）
                action.action_type = SyncActionType::Skip;
                action.reason = Some("删除前复核：云端仍存在该文件，跳过删除（cloud_tree 疑似残缺）".into());
                tracing::warn!(rel = ?action.relative_path, fid = %file_id, "删除前复核拦截误删");
            }
            Err(e) if e.is_not_found() => {
                // 云端确实不存在 → 允许删除（保持原动作）
            }
            Err(e) => {
                // 复核请求本身失败（网络问题）→ 保守起见跳过删除，下轮再判
                action.action_type = SyncActionType::Skip;
                action.reason = Some(format!("删除前复核请求失败，保守跳过：{e}"));
                tracing::warn!(rel = ?action.relative_path, "删除前复核失败，跳过删除");
            }
        }
    }
}
```

**为什么这样**：
- 与现有 `validate_delete_from_cloud`（L1749）对称，最小认知负担。
- 只对**有真实 fileId 的文件**复核（占位符、孤儿无 id 的不查，省 API 调用）。
- 复核请求失败时**保守跳过删除**（宁可漏删一次也不误删），下次 BFS 成功后再判。
- 这是"不可逆操作前的最后一次确认"，API 调用开销可接受（仅 DeleteFromLocal 动作触发，正常同步中很少）。

### 1.3 三道防御的协同

| 场景 | 第一道 | 第二道 | 第三道 |
|---|---|---|---|
| 正常同步（BFS 成功） | 写 complete=true | 不触发（非启动） | get_file 确认不存在→允许删 |
| BFS 部分失败但运行中 | 写 complete=false（不固化残缺） | 不触发（非启动） | 误删动作被复核拦截 |
| 崩溃后重启加载残缺缓存 | （上次已写 false→强制重跑） | 启动期 Skip 拦截 | 即使第二道漏了，复核兜底 |
| 三道全失效（极端） | — | — | 仍可能误删，但概率极低 |

---

## 2. BUG 3 — 崩溃闪退（保留 abort，仅修 icon 源）

### 2.1 根因（运行时实锤）

```
崩溃日志：muda-0.19.3/src/platform_impl/macos/icon.rs:34
  payload: called `Result::unwrap()` on `Err`: Format(FormatError { inner: ZeroWidth })
崩溃时刻：sync cycle 无操作后 7ms → 状态桥接 refresh_menu → muda 渲染图标 panic
放大器：Cargo.toml:112 panic = "abort" → 任意 panic 全进程 SIGABRT 闪退
```

崩溃路径：`refresh_menu`（tray.rs:247）→ `tray.set_menu`（tray.rs:270）→ muda 内部 `menuitem_set_icon`（mod.rs:1161）→ `icon.inner.to_nsimage`（icon.rs:42）→ `to_png`（icon.rs:25）→ `encoder.write_header().unwrap()`（icon.rs:34）→ ZeroWidth panic。

### 2.2 设计

按用户决策"保留 abort，仅修 icon 源"。由于崩溃发生在**第三方库 muda 内部**，应用层无法直接 try/catch（`panic=abort` 下 `catch_unwind` 也无效），修复策略是**消除触发条件 + 降低触发频率**，分两层。

#### 层一：消除 muda icon 渲染的触发源

**问题定位**：PetalLink 的 `MenuItem::with_id`（tray.rs:77/85/86/151/167）全部传 `None::<&str>`（无图标），理论上不该走 `menuitem_set_icon`。但 muda/tauri 在 `set_menu` 重建时，内部对菜单项的图标字段处理存在已知问题（ZeroWidth 意味着某个内部回退/默认图标解码出 width=0）。

**修复方向**（按可控性排序，实施时择优）：

1. **降低 refresh_menu 重建频率**（应用层可控，最稳妥）：
   - 当前 `refresh_menu`（tray.rs:247）在**无活跃传输时不节流**，每轮 `sync_state` 广播都重建（状态桥接 commands.rs:296）。崩溃前连续多轮"无操作短路"仍触发重建。
   - 修复：`refresh_menu` 增加"内容是否变化"判定——只有"正在传输"段的项目数/状态相比上次真正变化时才重建。维护一个 `last_transfer_signature: AtomicU64`（项目数 + 状态摘要 hash），相同时直接 return。这样无传输状态下菜单**根本不重建**，从源头消除 muda 渲染机会。
   - 同步状态变化（tooltip）走独立的 `update_tooltip`（tray.rs:279），不触发 menu 重建。

2. **图标加载防御**（tray.rs:48）：当前托盘图标 `Image::from_bytes(MENUBAR_ICON_PNG)` 失败时回退 `default_window_icon()`。确保回退路径也加尺寸校验——若解码出的图标 width/height 为 0，记录并跳过，不传入 muda。虽然崩溃点不是托盘图标本身，但消除所有"0 尺寸图标传入 muda"的可能路径。

3. **升级 tauri/muda**（如可行）：检查 tauri 2.11.3 之后版本或 muda 0.19.3 之后版本是否修复了该 icon panic（`write_header().unwrap()` 改为返回 Result）。若上游已修，升级是最干净的解法。实施时先验证升级兼容性。

#### 层二：abort 模式下的最后防线

由于保留 `panic="abort"`，一旦 muda 内部仍 panic，进程仍会闪退。作为"修 icon 源"策略的必要补充：

**修复**：在 `main.rs` 的 panic hook（L17-31）中，在 abort 前将崩溃信息**额外写入一个独立的 crash 标记文件**（如 `<support_dir>/last_crash.marker`），下次启动时检测该标记，向用户提示"上次异常退出"并可选择上报。这不阻止闪退，但让闪退可观测、可诊断（当前 hook 虽打日志，但用户不易发现）。

> 说明：用户明确选择"保留 abort"，故本方案不以改 panic 策略为目标。但需告知风险——只要 muda 仍有未发现的 panic 路径，闪退可能复发。层一的频率降低是主要防线，层二是可观测性补充。

### 2.3 与 BUG 4 的关系

BUG 3 的崩溃是 BUG 4 数据丢失的**触发条件之一**（崩溃→重启→加载残缺缓存）。修好 BUG 4 的三道防御后，即便仍崩溃，重启后也不会误删文件。两个 BUG 修复相互独立但协同增强稳健性。

---

## 3. BUG 2 — 上传失败无提示且无法重试（单任务重试 + 真断点续传 + toast）

### 3.1 根因

| 子问题 | 根因 | 位置 |
|---|---|---|
| 无提示 | 自动上传失败只 `warn!`，不发 toast；`transfer_update` 是空 payload，前端只 loadAll 不弹窗 | executor.rs:516-532；main.ts:51-54 |
| 无法重试 | 传输面板失败项纯展示无按钮；唯一 `sync_retry_failed` 重置 sync_items 表，不碰 transfer_queue | TransferPopover.vue:90-112；engine.rs:1820-1834 |
| 进度归零 | `settle_transfer` 失败时 size=0，transferred 被刷成 0 | executor.rs:301-314 |
| 重传从头 | `do_upload` 调 `upload()` 而非 `upload_resume(..., Some(&session))`，断点信息只在启动中断恢复才用 | executor.rs:486/507 |

### 3.2 设计

#### 3.2.1 失败 toast 提示

**位置**：`src/sync/executor.rs`（do_upload 失败分支，L516-532）+ 前端 `app/main.ts`

**后端**：在 `do_upload` 失败时，通过一个新的 Tauri 事件 `upload_failed` 携带结构化信息广播：

```rust
// executor.rs do_upload 失败分支
Err(e) => {
    tracing::warn!(rel = %rel, error = %e, "上传失败");
    // 新增：广播失败事件（含文件名 + 错误），前端据此弹 toast
    if let Some(app) = &self.app_handle {
        let _ = app.emit("upload_failed", serde_json::json!({
            "rel_path": rel,
            "error": e.to_string(),
        }));
    }
    ActionResult { success: false, error_message: Some(e.to_string()), ... }
}
```

**前端**：`app/main.ts` 监听 `upload_failed`，调用 toast（error 变体）显示"上传失败：<文件名>"。点击 toast 可展开传输队列面板。注意去重（同一文件短时间内只弹一次，避免重试风暴刷屏）。

#### 3.2.2 失败进度保留

**位置**：`src/sync/executor.rs`（`settle_transfer`，L282-321）

**修复**：失败结算时不把 `transferred` 刷成 0。当前 L301-308 在 `!result.success` 时 `size=0`，导致 L311-314 的 SQL 把 transferred 写成 0。改为：失败时保留实际已传字节数（从上传回调累计的值），让用户看到"已传 99%"而非"0%"，也为断点续传提供视觉依据。

#### 3.2.3 传输队列单任务重试 + 真断点续传

这是核心改动，涉及后端命令 + executor 路径 + 前端按钮。

**后端**：

1. **新增命令 `transfer_retry(task_id)`**（`src/commands.rs`）：
   - 从 `transfer_queue` 读取该 FAILED 行（含 rel_path、direction、已传字节、断点信息）。
   - 重置状态为 RUNNING。
   - 调用 executor 的重试路径（传入断点信息）。

2. **transfer_queue schema 扩展**（`src/data/`，需 migration v4）：新增字段持久化断点续传所需信息：
   - `session_url TEXT`（resume 会话 URL）
   - `upload_id TEXT`（会话 uploadId）
   - `slice_size INTEGER`（分片大小）
   - `last_offset INTEGER`（最后成功上传的 offset）
   
   上传过程中通过 `on_resume` 回调（executor.rs:442-450 已有写 DB 逻辑）持续更新这些字段。

3. **executor 重试路径**（`src/sync/executor.rs`）：新增 `retry_upload(task)` 方法：
   - 从 transfer_queue 读取断点信息（session_url、last_offset）。
   - 调用 `upload_api.upload_resume(..., Some(&session), last_offset)` 而非 `upload()`，**从断点继续**而非从头传。
   - `upload_api.rs` 的 `upload_resume` 已支持 resume 参数（L349-495），需确认从指定 offset 续传的接口（当前是 init→分片循环，需增加"跳过已传分片"逻辑：根据 last_offset 计算起始分片）。

4. **upload_api.rs 增强**（`src/drive/upload_api.rs`）：`upload_resume` 增加"从指定 offset 续传"能力：
   - `UploadResumeSession` 结构体扩展，新增 `session_url: Option<String>` 和 `start_offset: u64` 字段。
   - 若 `resume_session.session_url` 为 `Some`，跳过 init 步骤（`init_resume_session`，L497），直接复用已有 session_url。
   - 分片循环（L406-463）的起始 offset 改为 `resume_session.start_offset` 而非 0：`start_chunk = start_offset / slice_size`，循环变量初始化为该值。
   - 续传前**必须**先调 `PUT bytes */total`（`query_final_status`，L484）查询服务端已接收范围（rangeList），以服务端 rangeList 的最大 offset 为准对齐 `start_offset`（防止本地记录与服务端不一致导致分片重叠/空洞）。

**前端**：

5. **TransferPopover.vue 失败项加重试按钮**（L90-112）：
   - 失败项（state=FAILED）右侧增加"重试"图标按钮。
   - 点击调 `transfer_retry(task.id)`。
   - 重试中状态变回 RUNNING，进度从断点处继续显示。

6. **api/transfer.ts** 新增 `transferRetry(taskId)` 封装。

#### 3.2.4 与现有 sync_retry_failed 的关系

- `sync_retry_failed`（SyncStatusBar 按钮）保留，处理 **sync_items 表**的批量失败（同步引擎层面的失败）。
- `transfer_retry`（传输面板按钮）新增，处理 **transfer_queue 表**的单任务失败（用户主动发起/大文件传输失败）。
- 两者数据源不同，互不干扰。建议在 `transfer_retry` 成功后，同步刷新对应 sync_items 记录状态，保持一致。

---

## 4. REQ-1 — 睡眠/断网时暂停一切同步，仅做网络检测

### 4.1 根因

当前系统**完全没有网络状态感知**：
- `start_cloud_refresh_timer`（engine.rs:561-585）：纯 `tokio::sleep` 循环，每 `poll_interval_secs`（默认 60s）无脑触发 `run_auto_cloud_refresh`，**不检查网络**。
- watcher 触发的 `run_sync_cycle`、手动刷新同理，网络断了也照常发起 BFS / changes API / 上传。
- 无 macOS 睡眠/唤醒监听。

**后果链**（运行时实锤）：凌晨网络断开 → 定时器照常触发 → BFS 子目录连续失败 3 次（7/08 00:38-01:05 实锤）→ 写出 `complete=true` 但 `files=0` 的残缺缓存（BUG 4 根因）→ 后续误删。睡眠唤醒后也可能触发同样的陈旧状态误判。

> 即：**REQ-1 的修复同时是 BUG 4 的上游预防**——从源头消除"网络断开时瞎跑 BFS"。

### 4.2 设计

新增一个**网络与电源状态守卫模块**，作为所有同步操作的统一前置门控。

#### 4.2.1 网络连通性检测模块

**新增文件**：`src/core/net_guard.rs`

核心能力：
- 维护一个全局 `AtomicU8` 网络状态（`Online` / `Offline`），通过轻量探测维护。
- `is_online() -> bool`：供同步引擎各入口快速查询，零开销。
- `wait_until_online()`：异步等待网络恢复（供定时器循环使用，替代盲目 sleep）。

**网络状态判定策略**（两层）：

1. **被动监听（首选，零开销）**：监听 macOS 系统网络状态变化通知。通过 `objc2`/`SCNetworkReachability` 或更简单的方案——监听 `NSWorkspace.didWakeNotification`（睡眠唤醒）+ 周期性轻量 TCP 探测。考虑到实现复杂度，**采用探测为主、事件为辅**的混合方案（见下）。

2. **主动探测（兜底，权威）**：定时（如每 30s）向华为 API 域名做一次**极轻量 TCP 连接探测**（只 connect 不发数据，超时 3s）。目标 host 取 `DRIVE_API_BASE` 的主机名。这比完整 HTTP 请求轻得多，且能准确反映"到云端的连通性"。

**为什么用 TCP 探测而非 HTTP**：
- HTTP 探测要建 TLS + 发请求 + 收响应，重；TCP connect 只需握手，3s 超时，极轻。
- 目标是"网络是否通到华为云"，TCP connect 到 443 端口足够准确。
- 探测失败 → Offline；连续 1 次成功 → Online（保守地快速恢复）。

#### 4.2.2 睡眠/唤醒监听

**位置**：`src/core/net_guard.rs` + `src/lib.rs`（setup 注册）

监听 macOS 系统睡眠/唤醒通知：
- **睡眠时**（`NSWorkspaceWillSleepNotification`）：立即将状态置为 `Offline`（语义等同断网——睡眠期间不做任何操作），暂停所有同步。
- **唤醒时**（`NSWorkspaceDidWakeNotification`）：不立即置 Online，而是触发一次主动探测——唤醒后网络通常需要几秒重建（Wi-Fi 重连），探测确认恢复后才置 Online，恢复同步。

实现用 `objc2-app-kit` 的 `NSWorkspace` 通知中心（项目已依赖 `objc2-app-kit 0.3`，features 已含 `NSWorkspace`）。在 `lib.rs` setup 中注册观察者，回调里更新 `AtomicU8` 状态。

#### 4.2.3 同步引擎门控接入

**位置**：`src/sync/engine.rs`（三个入口）+ `src/mount/local_watcher.rs`

在以下入口增加 `if !net_guard::is_online() { return skip; }` 前置检查：

| 入口 | 位置 | 门控行为 |
|---|---|---|
| 云端定时刷新 | `start_cloud_refresh_timer`（L571-584） | 循环改为 `wait_until_online().await` 替代盲目 sleep；Offline 期间完全不触发 BFS |
| watcher 同步 | `run_sync_cycle`（L588） | Offline 时跳过（本地文件变更可累积，网络恢复后一并处理） |
| 增量 changes | `run_auto_cloud_refresh_impl` | Offline 时直接返回，不调 changes API |

**本地 watcher 特殊处理**：网络断开时，FSEvents 监听**保持运行**（本地变更仍需记录，避免遗漏），但触发的同步 cycle 在 engine 层被门控跳过。本地变更在 DB/内存中累积，网络恢复后首个 cycle 统一处理。

#### 4.2.4 定时器循环改造（核心）

```rust
// engine.rs start_cloud_refresh_timer 改造
tokio::spawn(async move {
    loop {
        if *engine.shutdown.lock() { break; }
        // ★ 替代盲目 sleep：等到网络在线 + 间隔到期
        // wait_until_online 内部是轻量探测循环（每 30s 探测），网络断开时阻塞在此
        net_guard::wait_until_online(&engine.shutdown).await;
        if *engine.shutdown.lock() { break; }
        tokio::time::sleep(Duration::from_secs(engine.poll_interval_secs as u64)).await;
        if *engine.shutdown.lock() { break; }
        // 二次确认（sleep 期间网络可能又断了）
        if !net_guard::is_online() {
            tracing::info!("网络离线，跳过本次云端刷新");
            continue;
        }
        engine.run_auto_cloud_refresh().await;
    }
});
```

**关键**：`wait_until_online` 接收 shutdown 信号，确保退出时能立即唤醒返回，不卡住。

#### 4.2.5 网络状态前端感知（可选增强）

广播一个 `network_state` 事件（online/offline），前端 SyncStatusBar 显示"网络未连接"提示，让用户知晓同步暂停原因。这提升体验但非必须，可作为可选项。

### 4.3 与 BUG 3/4 的协同

- **BUG 4**：REQ-1 从源头消除"断网时瞎跑 BFS"，残缺 cloud_tree 不再产生（上游预防）。
- **BUG 3**：网络断开时定时器不再触发 `run_auto_cloud_refresh` → 不广播 `sync_state` → `refresh_menu` 不被频繁调用 → 降低 muda icon panic 概率（频率降低）。

---

## 5. REQ-2 — 日志导出加固（保证导出全部）

### 5.1 现状与诊断困境

`logs_export`（commands.rs:1698-1720）逻辑：`read_dir(log_dir) → sort → 逐个 read_to_string 拼接 → write`。

**实测**：用 Rust 运行等价逻辑，当前能正确读入 12 个文件、拼接 4.2MB，导出全部 14 天。代码与 v1.0.7 二进制一致（`git diff` 为空）。

**无法复现**"导出只有当天"的现象。可能的原因（无法证实）：历史某次日志目录确实只剩当天文件后被恢复、或观察的是查看页列表而非导出文件。但用户诉求明确：**无论根因，导出必须保证全部**。

### 5.2 设计：加固导出健壮性 + 可观测性

既然根因待定，采取**多管齐下的加固**，覆盖所有可能的失败路径：

#### 5.2.1 导出诊断日志（可观测性，定位根因的关键）

在 `logs_export` 中增加 `tracing` 诊断，记录实际读到了几个文件、每个文件大小、最终拼接大小。下次用户复现时，可从日志里直接看到 `read_dir` 到底返回了几个文件——若返回 1 个，说明运行时 `log_dir` 指向的目录确实只有 1 个文件（路径分歧/清理），从而定位真正根因。

```rust
pub fn logs_export(path: String) -> AppResult<()> {
    let dir = crate::core::logging::log_dir()?;
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    // ★ 诊断：记录实际读到的文件，定位"只有当天"根因
    tracing::info!(
        dir = %dir.display(),
        count = files.len(),
        files = ?files.iter().map(|f| f.file_name().unwrap_or_default().to_string_lossy().to_string()).collect::<Vec<_>>(),
        "logs_export 开始导出"
    );
    // ... 原拼接逻辑 ...
    tracing::info!(out_bytes = out.len(), file_count = files.len(), "logs_export 完成");
    std::fs::write(&path, out)?;
    Ok(())
}
```

#### 5.2.2 导出健壮性加固

针对所有可能的失败路径加固：

1. **目录不存在/为空**：当前返回 `Err("日志目录为空")`。改为：若目录不存在，先记录警告并尝试创建（可能首次运行），仍为空才报错。

2. **read_to_string 对非 UTF-8 失败会静默跳过该文件**（`if let Ok(...)` 丢了 Err）：当前若某日志文件含非法 UTF-8（极端情况），会被静默跳过，用户不知道少了内容。改为：记录哪些文件读取失败；对失败文件用 `read`（字节）+ `from_utf8_lossy` 容忍解码，确保不丢内容。

3. **滚动文件过滤**：`read_dir` 会返回目录下**所有**文件。若 logs 目录混入非日志文件（如 `.DS_Store`），当前会被一起读入。增加前缀过滤：只处理 `PetalLink.log` 开头的文件，避免污染导出内容。

4. **日志数量上限保护**（预防性）：`tracing-appender 0.2.5` 的 `daily` 滚动**无 `with_max_log_files`**（该 API 在 0.2.x 不存在），旧日志会无限累积。虽当前是"存太多"而非"只存一天"，但长期会占空间。增加启动时的清理逻辑：保留最近 N 天（如 30 天）的日志文件，超出删除。这也防止目录膨胀拖慢导出。

#### 5.2.3 导出内容增强（可选）

导出文件开头增加一段元信息头，方便排查：
```
===== PetalLink 日志导出 =====
导出时间：2026-07-09 10:00:00
文件数：12
总大小：4294 KB
日期范围：2026-06-26 ~ 2026-07-09
==============================
===== PetalLink.log.2026-06-26 =====
（日志内容...）
```

### 5.3 验证策略

由于无法稳定复现，加固后的验证依赖**诊断日志**：下次用户导出时若仍只有当天，查看 `logs_export 开始导出` 那条诊断日志的 `count` 和 `files` 字段——
- 若 `count=1`：确认运行时 `log_dir` 目录确实只有 1 个文件，根因在文件系统/路径，进一步查为何其他文件消失。
- 若 `count=12` 但导出仍只有当天：说明拼接后写入出问题（极不可能，但诊断会排除）。

---

## 6. 实施顺序与依赖

按风险与依赖关系排序：

```
第一阶段（上游预防 + 数据安全，最高优先）：
  1. REQ-1 net_guard 模块（网络检测 + 睡眠监听）+ 定时器门控
     → 从源头消除"断网时瞎跑 BFS"，是 BUG 4 的上游预防
  2. BUG 4 第一道：cloud_tree BFS 完整性诚实化
  3. BUG 4 第二道：planner 启动恢复期删除守卫
  4. BUG 4 第三道：validate_delete_from_local 云端复核
  → 三道可并行开发，互不依赖，合并后立即生效

第二阶段（崩溃止血）：
  5. BUG 3 层一：refresh_menu 频率降低 + 图标防御
  6. BUG 3 层二：panic hook crash 标记

第三阶段（日志导出加固，独立小改动）：
  7. REQ-2 logs_export 诊断日志 + 健壮性加固 + 日志数量上限

第四阶段（上传体验）：
  8. BUG 2 失败进度保留 + toast（小改动，先做）
  9. BUG 2 transfer_queue schema v4 migration（断点字段）
  10. BUG 2 单任务重试 + 真断点续传（核心改动）
  11. BUG 2 前端重试按钮
```

> REQ-1 提到第一阶段首位，因为它是 BUG 3/4 的上游触发源——先堵住"断网瞎跑"，能显著降低后续 BUG 的触发概率，也让 BUG 4 三道防御的压力减轻。

## 7. 测试策略

### 7.1 BUG 4 测试（最关键）

- **单元测试**：
  - planner：构造 `is_startup_resume=true` + `local有/云端无/DB有真实id/未改` → 断言生成 Skip 而非 DeleteFromLocal。
  - planner：同条件但 `is_startup_resume=false`（会话内）→ 断言仍可生成 DeleteFromLocal（不破坏正常功能）。
  - cloud_tree：mock BFS 子树失败 → 断言不写 complete=true。
  - validate_delete_from_local：mock get_file 返回 Ok → 断言动作变 Skip。

- **集成测试**（wiremock）：
  - mock BFS list 子目录返回 500 连续 3 次 → 断言持久化文件 complete=false。
  - mock get_file 返回 200 → 断言 DeleteFromLocal 被拦截。

### 7.2 REQ-1 测试（网络门控）

- 单元测试：net_guard `is_online()` 在探测成功/失败时状态正确翻转。
- 集成测试：mock 网络离线 → 断言 `run_auto_cloud_refresh` 不被触发；mock 恢复 → 断言恢复触发。
- 手动测试：关闭 Wi-Fi → 观察日志无 BFS/changes 调用；睡眠 → 唤醒 → 观察先探测后恢复同步。

### 7.3 BUG 3 测试

- 构造高频 sync_state 广播场景 → 断言 refresh_menu 不重建（signature 未变）。
- 验证托盘图标加载防御：传入 0 尺寸图标 → 不 panic、降级处理。

### 7.4 REQ-2 测试

- 单元测试：logs_export 读取多文件场景（mock 目录）→ 断言拼接全部；含非 UTF-8 文件 → 断言 from_utf8_lossy 不丢内容；含 .DS_Store → 断言被过滤。
- 手动验证：实际导出，检查诊断日志的 count 字段。

### 7.5 BUG 2 测试

- 单元测试：settle_transfer 失败时 transferred 保留非 0。
- 集成测试：mock 上传分片在第 5 片失败 → transfer_retry → 断言从第 5 片续传（upload_resume 收到正确 offset）。
- 前端测试：TransferPopover 失败项渲染重试按钮，点击触发 transfer_retry。

## 8. 风险与回退

| 风险 | 缓解 |
|---|---|
| REQ-1 TCP 探测误判（防火墙/代理） | 探测目标用华为 API 域名（同步目标本身），端口 443；探测失败保守置 Offline 不影响数据安全，网络真恢复后快速恢复 |
| REQ-1 睡眠监听 FFI 复杂度 | objc2-app-kit NSWorkspace 通知项目已依赖；若 FFI 不稳定，退化为纯定时探测（功能降级但不阻塞） |
| validate_delete_from_local 增加 API 调用拖慢同步 | 仅对真实 fileId 的 DeleteFromLocal 触发；正常同步中删除动作少 |
| transfer_queue schema v4 migration 失败 | migration 加事务 + 旧字段兼容（NULL 容忍）；回退到 v3 仅丢失断点信息，不影响基础功能 |
| refresh_menu 频率降低导致传输状态更新不及时 | 传输段变化（项目数/状态）仍立即重建，仅"无变化"时跳过；tooltip 独立更新 |
| 日志数量上限清理误删 | 仅删超 30 天的 `PetalLink.log.*` 文件，保留近期；清理前记录日志 |

## 9. 不在本次范围

- BUG 5（云端副本）：随 BUG 4 修复解决，不单独处理。
- panic 策略调整：用户明确选择保留 abort，不改动 Cargo.toml:112。
- REQ-2 的"网络状态前端提示"：列为可选项，非必须。
