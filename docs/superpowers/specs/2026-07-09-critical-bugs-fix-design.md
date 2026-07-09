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

> BUG 1（日志只存一天）：代码与磁盘均显示有 14 天日志、无删除逻辑，疑为 dev/release 目录混淆或观察的是内存缓冲（`logs_list` 只返回最近 1000 条）。**本方案不纳入修复**，先由用户按 dev/release 目录区分复现确认；若确认确有问题再单独定位。

> BUG 5（莫名出现云端副本）：经查证，这些"云端副本"是华为官方客户端在 2025 年生成的历史冲突副本，原本就存在于云端，PetalLink 只是如实镜像。其"删除又重现"是 BUG 4 的次生症状，随 BUG 4 修复一并解决。

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

## 4. 实施顺序与依赖

按风险与依赖关系排序：

```
第一阶段（数据安全，最高优先）：
  1. BUG 4 第一道：cloud_tree BFS 完整性诚实化
  2. BUG 4 第二道：planner 启动恢复期删除守卫
  3. BUG 4 第三道：validate_delete_from_local 云端复核
  → 三道可并行开发，互不依赖，合并后立即生效

第二阶段（崩溃止血）：
  4. BUG 3 层一：refresh_menu 频率降低 + 图标防御
  5. BUG 3 层二：panic hook crash 标记
  → 依赖第一阶段完成验证后进行

第三阶段（上传体验）：
  6. BUG 2 失败进度保留 + toast（小改动，先做）
  7. BUG 2 transfer_queue schema v4 migration（断点字段）
  8. BUG 2 单任务重试 + 真断点续传（核心改动）
  9. BUG 2 前端重试按钮
```

## 5. 测试策略

### 5.1 BUG 4 测试（最关键）

- **单元测试**：
  - planner：构造 `is_startup_resume=true` + `local有/云端无/DB有真实id/未改` → 断言生成 Skip 而非 DeleteFromLocal。
  - planner：同条件但 `is_startup_resume=false`（会话内）→ 断言仍可生成 DeleteFromLocal（不破坏正常功能）。
  - cloud_tree：mock BFS 子树失败 → 断言不写 complete=true。
  - validate_delete_from_local：mock get_file 返回 Ok → 断言动作变 Skip。

- **集成测试**（wiremock）：
  - mock BFS list 子目录返回 500 连续 3 次 → 断言持久化文件 complete=false。
  - mock get_file 返回 200 → 断言 DeleteFromLocal 被拦截。

### 5.2 BUG 3 测试

- 构造高频 sync_state 广播场景 → 断言 refresh_menu 不重建（signature 未变）。
- 验证托盘图标加载防御：传入 0 尺寸图标 → 不 panic、降级处理。

### 5.3 BUG 2 测试

- 单元测试：settle_transfer 失败时 transferred 保留非 0。
- 集成测试：mock 上传分片在第 5 片失败 → transfer_retry → 断言从第 5 片续传（upload_resume 收到正确 offset）。
- 前端测试：TransferPopover 失败项渲染重试按钮，点击触发 transfer_retry。

## 6. 风险与回退

| 风险 | 缓解 |
|---|---|
| validate_delete_from_local 增加 API 调用拖慢同步 | 仅对真实 fileId 的 DeleteFromLocal 触发；正常同步中删除动作少 |
| transfer_queue schema v4 migration 失败 | migration 加事务 + 旧字段兼容（NULL 容忍）；回退到 v3 仅丢失断点信息，不影响基础功能 |
| refresh_menu 频率降低导致传输状态更新不及时 | 传输段变化（项目数/状态）仍立即重建，仅"无变化"时跳过；tooltip 独立更新 |
| muda 升级引入新问题 | 升级前在 dev 环境完整回归；保留层一频率降低作为不依赖升级的主防线 |

## 7. 不在本次范围

- BUG 1（日志）：待用户确认复现环境后再定。
- BUG 5（云端副本）：随 BUG 4 修复解决，不单独处理。
- panic 策略调整：用户明确选择保留 abort，不改动 Cargo.toml:112。
