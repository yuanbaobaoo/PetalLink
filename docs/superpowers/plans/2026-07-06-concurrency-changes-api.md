# 并发模型增强 + 华为 changes API 接入 — 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 分三阶段实现：①自动刷新跳过索引 + ②task 可见性（阶段一）；④-A 独立 binary 验证华为 `/drive/v1/changes` 接口（阶段二）；④-B 基于 results 接入 changes API 实现增量同步（阶段三）。

**Architecture:** "线程"= tokio async task（沿用项目惯例，不引入 OS 线程）。阶段一改 `run_auto_cloud_refresh` 加 `is_indexing` 检查；阶段二新建 `src/bin/changes_probe.rs` 探查接口；阶段三新建 `src/drive/changes_api.rs` + cursor 持久化 + 改造自动刷新为"有 cursor 增量/无 cursor 全量"，失败回退全量 BFS。

**Tech Stack:** Rust + Tauri 2 + tokio（async task）+ reqwest + rusqlite + serde_json

**关联文档：** 设计 spec `docs/superpowers/specs/2026-07-06-concurrency-changes-api-design.md`

---

## 文件结构

**阶段一（仅 1 文件）：**
- Modify: `src/sync/engine.rs` — `run_auto_cloud_refresh` 加 `is_indexing` 检查

**阶段二（仅 1 文件）：**
- Create: `src/bin/changes_probe.rs` — 独立 dev binary 探查 changes 接口

**阶段三（4 文件）：**
- Create: `src/drive/changes_api.rs` — `ChangesApi` + `Change`/`ChangeKind`/`ChangeListResult` 模型 + `list_changes` + 自动分页
- Modify: `src/drive/mod.rs` — 注册 `pub mod changes_api`
- Modify: `src/core/cache_paths.rs` — 加 `changes_cursor_file()` 路径函数
- Modify: `src/sync/engine.rs` — `run_auto_cloud_refresh_impl` 改造为增量/全量自动切换；新增 `CHANGES_API` 字段或在调用处构造

> **重要：阶段三的具体字段映射（cursor 名、change 数组名、removed 标志）依赖阶段二的验证报告。阶段三任务里给出了基于 GDrive 协议的合理默认值，执行阶段三前必须先看阶段二填写的 spec 第 6 节，按真实接口行为调整 `ChangeListResult::from_json` 的字段名。**

---

# 阶段一：自动刷新跳过索引 + task 可见性

## Task 1: 自动刷新显式检查 `is_indexing` 跳过

**Files:**
- Modify: `src/sync/engine.rs`（`run_auto_cloud_refresh` 方法，约 1215 行）

- [ ] **Step 1: 在 `run_auto_cloud_refresh` 开头加 `is_indexing` 检查**

定位 `run_auto_cloud_refresh` 方法（`async fn run_auto_cloud_refresh(self: &Arc<Self>)`），在 `// 与手动刷新互斥` 注释前插入检查。改为：

```rust
    async fn run_auto_cloud_refresh(self: &Arc<Self>) {
        // ①索引中（含手动刷新/启动 BFS/其他自动刷新的 BFS 阶段）→ 跳过本次，等下次定时
        if self.is_indexing() {
            tracing::info!("自动云端刷新: 索引进行中，跳过本次");
            return;
        }
        // 与手动刷新互斥：若手动刷新进行中，跳过本次自动刷新
        {
            let mut guard = self.manual_syncing.lock();
            if *guard {
                tracing::info!("自动云端刷新：手动同步进行中，跳过本次");
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

- [ ] **Step 2: 验证编译**

Run: `cargo build --lib`
Expected: `Finished` 无错误

- [ ] **Step 3: 运行测试确认无回归**

Run: `cargo test --lib`
Expected: `test result: ok. 186 passed; 0 failed`

- [ ] **Step 4: Commit**

```bash
git add src/sync/engine.rs
git commit -m "feat: 自动刷新在索引进行中显式跳过

run_auto_cloud_refresh 开头检查 is_indexing，索引中跳过本次并记日志。
比 manual_syncing 锁更宽——连启动期 BFS 未完成也能正确跳过，避免索引重叠。"
```

---

# 阶段二：验证华为 changes 接口

## Task 2: 新建 changes_probe 独立 binary

**Files:**
- Create: `src/bin/changes_probe.rs`

参考现有 `src/bin/upload_tester.rs` 的模式（`#[tokio::main]` + `dotenvy::dotenv()` + 从 env/OAuth 获取 token + `tracing_subscriber`）。本 binary 用裸 `reqwest` 直接打 `/drive/v1/changes`，不走 `DriveClient`（因为是探查未验证接口，要拿到原始响应）。

- [ ] **Step 1: 创建 binary 文件**

Create `src/bin/changes_probe.rs`：

```rust
//! 命令行探查工具 —— 华为 Drive /drive/v1/changes 接口行为。
//!
//! 用法:
//!   cargo run --bin changes_probe
//!
//! 自动获取 token：先尝试 HWCLOUD_TEST_TOKEN 环境变量，
//! 若无则启动 OAuth 授权流程（打开浏览器）。
//! 注：与 upload_tester 一样，无法读取主程序加密的 token.bin。

use petal_link_lib::auth::service::AuthService;

const DRIVE_API_BASE: &str = "https://drive.cloud.huawei.com.cn";

async fn get_token() -> String {
    if let Ok(t) = std::env::var("HWCLOUD_TEST_TOKEN") {
        if !t.is_empty() {
            eprintln!("✓ 从环境变量读取 token");
            return t;
        }
    }
    eprintln!("未找到 HWCLOUD_TEST_TOKEN，启动 OAuth 授权...");
    let auth = AuthService::new();
    match auth.authorize(9999).await {
        Ok(token_pair) => token_pair.access_token,
        Err(e) => {
            eprintln!("✗ OAuth 授权失败: {e}");
            eprintln!("  请: export HWCLOUD_TEST_TOKEN=\"<access_token>\"");
            std::process::exit(1);
        }
    }
}

/// 打印一次请求的完整信息：URL、状态码、响应体。
async fn probe(client: &reqwest::Client, token: &str, label: &str, url: &str) {
    eprintln!("\n═══════════════════════════════════════════════════════════");
    eprintln!("探测: {label}");
    eprintln!("URL : {url}");
    let resp = match client.get(url).bearer_auth(token).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("✗ 请求失败: {e}");
            return;
        }
    };
    let status = resp.status();
    eprintln!("状态: {status}");
    let body = resp.text().await.unwrap_or_else(|e| format!("<读取响应体失败: {e}>"));
    // 尝试 pretty-print JSON
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => eprintln!("响应:\n{}", serde_json::to_string_pretty(&v).unwrap_or(body)),
        Err(_) => eprintln!("响应(非JSON):\n{body}"),
    }
    eprintln!("═══════════════════════════════════════════════════════════");
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info,petal_link_lib=info")
        .init();

    let token = get_token().await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    // 1) 不带 cursor，首次拉取（看是否返回初始 cursor / 或报错要 cursor）
    probe(&client, &token, "1. 首次无 cursor", &format!("{DRIVE_API_BASE}/drive/v1/changes")).await;

    // 2) 带 fields=*（华为 /about 接口要求 fields=*，看 changes 是否同理）
    probe(&client, &token, "2. fields=* 无 cursor", &format!("{DRIVE_API_BASE}/drive/v1/changes?fields=*")).await;

    // 3.1) GDrive 协议: getStartPageToken 取初始游标
    probe(&client, &token, "3. getStartPageToken", &format!("{DRIVE_API_BASE}/drive/v1/changes/startPageToken")).await;

    // 3.2) 带 pageSize 限制（看分页字段名）
    probe(&client, &token, "4. pageSize=1", &format!("{DRIVE_API_BASE}/drive/v1/changes?fields=*&pageSize=1")).await;

    // 4) 带 cursor 重试（先用空字符串看报错信息，了解 cursor 参数名）
    probe(&client, &token, "5. cursor=空", &format!("{DRIVE_API_BASE}/drive/v1/changes?fields=*&cursor=")).await;

    eprintln!("\n✓ 探查完成。请把以上响应贴入设计文档第 6 节，用于阶段三字段映射。");
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo build --bin changes_probe`
Expected: `Finished` 无错误（如缺 reqwest/serde_json 依赖，主 crate 已含，binary 复用）

- [ ] **Step 3: 运行探查并记录结果**

前置：确保 `.env` 有 `HWCLOUD_CLIENT_ID`/`HWCLOUD_CLIENT_SECRET`，并提供一个有效 access token：
```bash
export HWCLOUD_TEST_TOKEN="<你的有效 access_token>"
cargo run --bin changes_probe
```

把输出贴入设计文档 `docs/superpowers/specs/2026-07-06-concurrency-changes-api-design.md` 第 6 节，逐项确认：
- 初始 cursor 获取方式
- 响应结构（数组名、字段名）
- 变更类型判定（removed 标志）
- cursor 持久性（cursor 值是否稳定可复用）
- 最终一致性表现

> 注：这一步需要真实 token + 联网，可能需要用户协助获取 token。若 token 过期，binary 会回退到 OAuth 重新授权。

- [ ] **Step 4: Commit**

```bash
git add src/bin/changes_probe.rs
git commit -m "feat: 新增 changes_probe binary 探查华为 /drive/v1/changes 接口

独立 dev 工具，用真实 token 探查 5 种参数组合，摸清接口行为
（cursor 获取、响应结构、变更类型、分页），为阶段三接入提供依据。"
```

---

# 阶段三：接入 changes API

> **执行前提：** 阶段二的验证报告已填入设计文档第 6 节。下面任务中的字段名（`changes`/`nextCursor`/`removed`）是基于 GDrive 协议的合理默认；若阶段二发现华为用不同字段名，执行前**先把下面 `from_json` 实现里的字段名替换成真实值**。

## Task 3: 新建 changes_api.rs 模块 + 模型

**Files:**
- Create: `src/drive/changes_api.rs`
- Modify: `src/drive/mod.rs`（注册模块）

- [ ] **Step 1: 在 `src/drive/mod.rs` 注册模块**

在 `pub mod about_api;` 后加一行：

```rust
pub mod changes_api;
```

- [ ] **Step 2: 创建 `src/drive/changes_api.rs`**

仿 `about_api.rs` 风格（用 `DriveClient::get`）。模型 `Change`/`ChangeKind`/`ChangeListResult` 仿 `models.rs` 的 `FileListResult::from_json`（多 key 探测容错）。

Create `src/drive/changes_api.rs`：

```rust
//! Changes API —— 华为 Drive 增量变更接口（GET /drive/v1/changes）。
//!
//! 用于自动云端刷新的增量路径：相比全量 BFS（refresh_cloud_tree）大幅省流量、提速。
//! cursor 持久化后可跨重启复用；失效或接口异常时由调用方回退全量 BFS。
//!
//! ⚠️ 字段名（changes/nextCursor/removed）基于 GDrive 协议推断 + 阶段二验证。
//!    若华为实际字段不同，调整 ChangeListResult::from_json 的键名探测。

use std::sync::Arc;

use crate::drive::client::{handle_error_response, DriveClient};
use crate::drive::models::DriveFile;
use crate::error::{AppError, AppResult};

/// 变更类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// 文件被移除（云端删除）。具体判定见 Change::from_json 注释。
    Removed,
    /// 文件新增或元数据修改（含内容更新、改名、移动）。
    Modified,
}

/// 单条变更：一个云端文件的增/改/删事件。
#[derive(Debug, Clone)]
pub struct Change {
    pub kind: ChangeKind,
    pub file: DriveFile,
}

/// 变更列表 + 分页游标。
#[derive(Debug, Clone)]
pub struct ChangeListResult {
    pub changes: Vec<Change>,
    /// 下一页游标；None 表示已追平最新（无更多变更）。
    pub next_cursor: Option<String>,
}

impl ChangeListResult {
    /// 从 JSON 解析。键名容错：nextCursor 优先，回退 cursor（对齐 FileListResult 惯例）。
    /// ⚠️ 字段名以阶段二验证报告为准，必要时调整。
    pub fn from_json(json: &serde_json::Value) -> Self {
        let changes = json
            .get("changes")
            .or_else(|| json.get("items")) // 回退名
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(Change::from_json).collect())
            .unwrap_or_default();

        let next_cursor = json
            .get("nextCursor")
            .or_else(|| json.get("newStartCursor"))
            .or_else(|| json.get("cursor"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        Self { changes, next_cursor }
    }
}

impl Change {
    /// 从单条 change JSON 解析。
    /// ⚠️ removed 判定以阶段二验证为准：GDrive 用 removed:true，华为可能用 fileDeleted 或其他。
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        // 删除判定：优先看显式标志，再看是否缺 file 元数据
        let is_removed = v
            .get("removed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || v.get("fileDeleted").and_then(|v| v.as_bool()).unwrap_or(false);

        let file = if is_removed {
            // 删除事件可能只带 fileId，构造最小 DriveFile（id 来自 fileId 字段）
            let id = v.get("fileId").and_then(|v| v.as_str())
                .or_else(|| v.get("id").and_then(|v| v.as_str()))?
                .to_string();
            DriveFile {
                id,
                name: String::new(),
                category: crate::drive::models::FileCategory::None,
                size: 0,
                parent_folder: None,
                description: None,
                created_time: None,
                edited_time: None,
                mime_type: None,
                content_hash: None,
                thumbnail_link: None,
            }
        } else {
            // 增/改事件：file 字段内是完整 DriveFile
            let file_json = v.get("file").unwrap_or(v);
            DriveFile::from_json(file_json)
        };

        Some(Self {
            kind: if is_removed { ChangeKind::Removed } else { ChangeKind::Modified },
            file,
        })
    }
}

pub struct ChangesApi {
    client: Arc<DriveClient>,
}

impl ChangesApi {
    pub fn new(client: Arc<DriveClient>) -> Self {
        Self { client }
    }

    /// 拉取一页增量变更（pageSize 默认 100）。
    pub async fn list_changes(&self, cursor: Option<&str>) -> AppResult<ChangeListResult> {
        let mut url = format!("/drive/v1/changes?fields=*&pageSize=100");
        if let Some(c) = cursor {
            if !c.is_empty() {
                url.push_str(&format!("&cursor={}", crate::drive::files_api::urlencoding(c)));
            }
        }
        let resp = self.client.get(&url).await?;
        if !resp.status().is_success() {
            return Err(handle_error_response(resp).await);
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::generic(format!("解析 changes 响应失败：{e}")))?;
        Ok(ChangeListResult::from_json(&body))
    }

    /// 拉取全部增量变更（自动分页至 next_cursor 为空）。最多 100 页兜底。
    pub async fn list_all_changes(&self, start_cursor: Option<&str>) -> AppResult<(Vec<Change>, Option<String>)> {
        const MAX_PAGES: usize = 100;
        let mut all = Vec::new();
        let mut cursor: Option<String> = start_cursor.map(|s| s.to_string());
        for _ in 0..MAX_PAGES {
            let result = self.list_changes(cursor.as_deref()).await?;
            all.extend(result.changes);
            cursor = result.next_cursor;
            if cursor.is_none() {
                return Ok((all, None));
            }
        }
        tracing::warn!("list_all_changes 超过 {MAX_PAGES} 页，截断；返回最后 cursor");
        Ok((all, cursor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_modified_change() {
        // 增/改事件：file 字段内是完整文件
        let json = serde_json::json!({
            "changes": [{
                "file": { "id": "f1", "fileName": "a.txt", "mimeType": "text/plain", "size": 100 }
            }],
            "nextCursor": "cur123"
        });
        let r = ChangeListResult::from_json(&json);
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].kind, ChangeKind::Modified);
        assert_eq!(r.changes[0].file.name, "a.txt");
        assert_eq!(r.next_cursor.as_deref(), Some("cur123"));
    }

    #[test]
    fn test_parse_removed_change() {
        // 删除事件：removed 标志 + fileId
        let json = serde_json::json!({
            "changes": [{ "removed": true, "fileId": "f9" }]
        });
        let r = ChangeListResult::from_json(&json);
        assert_eq!(r.changes.len(), 1);
        assert_eq!(r.changes[0].kind, ChangeKind::Removed);
        assert_eq!(r.changes[0].file.id, "f9");
        assert!(r.next_cursor.is_none());
    }

    #[test]
    fn test_parse_empty() {
        let json = serde_json::json!({ "changes": [] });
        let r = ChangeListResult::from_json(&json);
        assert!(r.changes.is_empty());
        assert!(r.next_cursor.is_none());
    }
}
```

- [ ] **Step 3: 验证编译 + 跑测试**

Run: `cargo test --lib drive::changes_api`
Expected: 3 tests pass

Run: `cargo build --lib`
Expected: `Finished`

- [ ] **Step 4: Commit**

```bash
git add src/drive/changes_api.rs src/drive/mod.rs
git commit -m "feat: 新增 ChangesApi 模块（华为增量变更接口封装）

- list_changes / list_all_changes（自动分页）
- Change/ChangeKind/ChangeListResult 模型，字段名容错
- 字段映射基于 GDrive 协议推断，待阶段二验证后校准"
```

## Task 4: cursor 持久化路径

**Files:**
- Modify: `src/core/cache_paths.rs`（加 `changes_cursor_file`）

- [ ] **Step 1: 加 cursor 文件路径函数**

在 `src/core/cache_paths.rs` 末尾（`cloud_tree_cache_file` 之后）加：

```rust
/// changes 增量游标缓存文件：`<base>/changes_cursor_<escaped>.txt`
///
/// 存放华为 /drive/v1/changes 的分页游标，跨重启复用以走增量路径。
/// cursor 失效或文件缺失 → 调用方回退全量 BFS。
pub fn changes_cursor_file(abs_mount_dir: &str) -> AppResult<PathBuf> {
    Ok(cache_base_dir()?.join(format!(
        "changes_cursor_{}.txt",
        escape_mount_path(abs_mount_dir)
    )))
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo build --lib`
Expected: `Finished`

- [ ] **Step 3: Commit**

```bash
git add src/core/cache_paths.rs
git commit -m "feat: changes cursor 持久化路径

changes_cursor_<escaped>.txt，存 Application Support 工作目录，
跨重启复用游标走增量；失效则回退全量 BFS。"
```

## Task 5: 改造 run_auto_cloud_refresh_impl 为增量/全量自动切换

**Files:**
- Modify: `src/sync/engine.rs`（`run_auto_cloud_refresh_impl`，约 1233 行）

这是阶段三核心：有 cursor 走增量（merge 进 cloud_tree），无/失效走全量 BFS。

- [ ] **Step 1: 在 engine 引入 ChangesApi 并改造 impl**

先在 `SyncEngine::new` 构造处（`src/commands.rs` 约 243 行 `SyncEngine::new(...)`）补传一个 `Arc<ChangesApi>`（参考现有 `files_api` 的传法）。或在 `run_auto_cloud_refresh_impl` 内就地构造 `ChangesApi::new(self.client.clone())`——但 engine 无 `client` 字段，需新增。**选最小改动：engine 新增 `changes_api: Arc<ChangesApi>` 字段 + 构造参数。**

(a) `src/sync/engine.rs` struct 加字段（在 `files_api` 附近）：
```rust
    files_api: Arc<FilesApi>,
    changes_api: Arc<crate::drive::changes_api::ChangesApi>,
```

(b) `SyncEngine::new` 签名加参数并在 Self 初始化：
```rust
    pub fn new(
        files_api: Arc<FilesApi>,
        changes_api: Arc<crate::drive::changes_api::ChangesApi>,
        download_api: Arc<DownloadApi>,
        // ... 其余不变
    ) -> Self {
        // ...
        Self { files_api, changes_api, download_api, ... }
    }
```

(c) `src/commands.rs` 的 `SyncEngine::new(...)` 调用处（约 243 行），在 `FILES_API.clone()` 后补 `Arc::new(crate::drive::changes_api::ChangesApi::new(DRIVE_CLIENT.clone()))`。**或在 commands.rs 顶部 Lazy 全局加 `CHANGES_API`（仿 `FILES_API`），更对齐现有惯例：**

在 `src/commands.rs` 全局单例区（`FILES_API` 旁）加：
```rust
pub static CHANGES_API: Lazy<Arc<ChangesApi>> =
    Lazy::new(|| Arc::new(ChangesApi::new(DRIVE_CLIENT.clone())));
```
（注意 import：`use crate::drive::changes_api::ChangesApi;`）

构造调用处传 `CHANGES_API.clone()`。

(d) 修复 engine.rs 内测试 helper（`SyncEngine::new(...)` 调用，约 1399 行）补传 changes_api 参数——测试里用 `Arc::new(ChangesApi::new(client.clone()))`。

- [ ] **Step 2: 改造 `run_auto_cloud_refresh_impl` 为增量优先 + 全量回退**

把现有 `run_auto_cloud_refresh_impl`（约 1233 行起）改为：

```rust
    /// 自动云端刷新实现：有 cursor 走增量 changes，无/失效走全量 BFS（持有 manual_syncing 锁时调用）。
    async fn run_auto_cloud_refresh_impl(&self) -> AppResult<()> {
        let mount_dir = self.mount_dir.lock().clone().unwrap_or_default();
        let abs_dir = mount_dir.replace("~/", &format!("{}/", std::env::var("HOME").unwrap_or_default()));

        // 广播 is_indexing=true
        {
            let mut st = self.state.lock().clone();
            st.is_indexing = true;
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }

        // 尝试增量（有持久化 cursor）；失败/无 cursor 回退全量 BFS
        let refresh_result = self.try_incremental_or_full_refresh(&abs_dir).await;

        // 无论成败复位 is_indexing
        {
            let mut st = self.state.lock().clone();
            st.is_indexing = false;
            *self.state.lock() = st.clone();
            let _ = self.state_tx.send(st);
        }

        refresh_result?;
        self.run_sync_cycle("auto-cloud-refresh").await?;
        Ok(())
    }

    /// 增量优先：有 cursor → changes API merge；失败/无 cursor → 全量 BFS。
    async fn try_incremental_or_full_refresh(&self, abs_dir: &str) -> AppResult<()> {
        let cursor_path = crate::core::cache_paths::changes_cursor_file(abs_dir)?;
        let saved_cursor = std::fs::read_to_string(&cursor_path)
            .ok()
            .filter(|s| !s.trim().is_empty());

        if let Some(ref cursor) = saved_cursor {
            // 增量路径
            match self.changes_api.list_all_changes(Some(cursor)).await {
                Ok((changes, new_cursor)) => {
                    tracing::info!(count = changes.len(), "增量 changes 拉取成功，merge 进 cloud_tree");
                    self.merge_changes_into_cloud_tree(&changes);
                    // 更新 cursor（None 表示已追平，保留旧 cursor 或写空均可；这里写新值或清空）
                    let to_write = new_cursor.as_deref().unwrap_or(cursor);
                    let _ = std::fs::write(&cursor_path, to_write);
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(error = %e, "增量 changes 失败，回退全量 BFS 并清 cursor");
                    let _ = std::fs::remove_file(&cursor_path);
                }
            }
        }

        // 全量 BFS 路径（无 cursor 或增量失败）
        let (tree, p2i, root) = cloud_tree::refresh_cloud_tree(&self.files_api, &self.mount, abs_dir).await?;
        *self.cloud_tree.lock() = tree;
        *self.path_to_id.lock() = p2i;
        *self.root_folder_id.lock() = root;
        // 全量成功后清旧 cursor，下次自动重新建立增量基线
        // （若 changes 接口支持「全量后取 startCursor」，此处可改为写入 startCursor；按阶段二验证调整）
        let _ = std::fs::remove_file(&cursor_path);
        Ok(())
    }

    /// 把增量 changes merge 进内存 cloud_tree（按 rel_path 增删改）。
    fn merge_changes_into_cloud_tree(&self, changes: &[crate::drive::changes_api::Change]) {
        use crate::drive::changes_api::ChangeKind;
        // 反查 path_to_id 找 rel_path（fileId → rel_path）；增量变更的 file 有 id 但可能无 path，
        // 这里用 path_to_id 反查；查不到的（新建文件）按 cloud_file 的 parentFolder 反查父路径拼接。
        // 简化实现：遍历 changes，按 fileId 反查 rel_path，命中则增删改 cloud_tree；未命中跳过（下次全量补）。
        let p2i = self.path_to_id.lock();
        let id_to_path: std::collections::HashMap<&String, &String> = p2i.iter().map(|(p, id)| (id, p)).collect();
        let mut tree = self.cloud_tree.lock();
        for c in changes {
            match c.kind {
                ChangeKind::Removed => {
                    if let Some(rel) = id_to_path.get(&c.file.id) {
                        tree.remove(*rel);
                    }
                }
                ChangeKind::Modified => {
                    // 已知路径：更新；未知路径（新建）：跳过，等下次全量 BFS 兜底
                    if let Some(rel) = id_to_path.get(&c.file.id) {
                        tree.insert((*rel).clone(), c.file.clone());
                    }
                }
            }
        }
    }
```

- [ ] **Step 3: 验证编译**

Run: `cargo build --lib`
Expected: `Finished`（若有 unused import / 类型不匹配，按提示修）

- [ ] **Step 4: 运行全量测试**

Run: `cargo test --lib`
Expected: 所有测试通过（含新增 changes_api 的 3 个）

- [ ] **Step 5: Commit**

```bash
git add src/sync/engine.rs src/commands.rs
git commit -m "feat: 自动刷新接入增量 changes API（增量优先，全量回退）

- run_auto_cloud_refresh_impl 改为：有 cursor → list_all_changes merge 进 cloud_tree；
  无 cursor/失败 → 全量 BFS + 清 cursor
- engine 新增 changes_api 字段；merge_changes_into_cloud_tree 按 fileId 反查 rel_path 增删改
- 增量失败自动回退全量，保证正确性"
```

## Task 6: 启动时建立 cursor 基线（可选优化）

**Files:**
- Modify: `src/sync/engine.rs`（`start()` 全量 BFS 后）

启动时的全量 BFS 完成后，若 changes 接口支持 startPageToken，写入 cursor 建立增量基线，使首次自动刷新即可走增量。

- [ ] **Step 1: 在 `start()` 全量 BFS 后尝试取 startCursor**

定位 `start()`（约 182 行）的 `self.load_or_refresh_cloud_tree(&mount_dir).await?;` 之后，加：

```rust
        // 全量 BFS 后尝试建立 changes 增量基线 cursor（失败静默，不影响启动）
        let _ = self.try_init_changes_cursor(&mount_dir).await;
```

新增辅助方法（engine impl 内）：

```rust
    /// 全量 BFS 后尝试取 changes startCursor 建立增量基线。失败静默。
    async fn try_init_changes_cursor(&self, mount_dir: &str) {
        let abs_dir = mount_dir.replace("~/", &format!("{}/", std::env::var("HOME").unwrap_or_default()));
        let cursor_path = match crate::core::cache_paths::changes_cursor_file(&abs_dir) {
            Ok(p) => p,
            Err(_) => return,
        };
        // 已有 cursor 则不重复初始化
        if cursor_path.exists() { return; }
        // 尝试取 startCursor（阶段二验证后调整字段名/端点）
        match self.changes_api.list_changes(None).await {
            Ok(r) => {
                if let Some(c) = r.next_cursor {
                    let _ = std::fs::write(&cursor_path, &c);
                    tracing::info!("已建立 changes 增量基线 cursor");
                }
            }
            Err(e) => tracing::debug!(error = %e, "取 changes startCursor 失败（忽略，下次自动刷新会全量回退）"),
        }
    }
```

> 注：startCursor 的确切取法以阶段二验证为准。若华为不支持，本任务可跳过——首次自动刷新会无 cursor 走全量 BFS，全量后由 `try_incremental_or_full_refresh` 的逻辑自然不写 cursor，下次仍全量。此时增量功能退化为「永远全量」，但功能不报错。

- [ ] **Step 2: 验证编译 + 测试**

Run: `cargo build --lib && cargo test --lib`
Expected: 全过

- [ ] **Step 3: Commit**

```bash
git add src/sync/engine.rs
git commit -m "feat: 启动全量 BFS 后建立 changes 增量基线 cursor

使首次自动刷新即可走增量路径；不支持 startCursor 时静默跳过，
功能退化为全量（不报错）。"
```

---

# 阶段三完成后的端到端验证

- [ ] **Step 1: cargo build + 全量测试**

```bash
cargo build --lib && cargo test --lib
```
Expected: 全过

- [ ] **Step 2: 手工验证增量路径**

1. `cargo tauri dev --config tauri.dev.conf.json`
2. 登录 + 配置挂载目录，完成首次全量同步（建立 cursor 基线）
3. 等一个自动刷新周期（或临时把 poll_interval_sec 设为 60），在网页端改/删一个文件
4. 观察日志：应出现 `增量 changes 拉取成功，merge 进 cloud_tree`，且本地文件随之变化
5. 检查 `~/Library/Application Support/io.github.yuanbaobaoo.PetalLink-dev/changes_cursor_*.txt` 存在且有值

- [ ] **Step 3: 手工验证全量回退**

1. 手动删除 cursor 文件
2. 触发自动刷新 → 应走全量 BFS（日志 `refresh_cloud_tree`），并重新建立 cursor
3. 或模拟 changes 接口失败（断网）→ 应回退全量 BFS + 清 cursor

---

## Self-Review（计划自审）

**1. Spec 覆盖：**
- ①跳过索引 → Task 1 ✓
- ②task 可见性 → Task 1（tracing 日志）✓
- ④验证接口 → Task 2 ✓
- ④接入 changes API（新模块 + cursor 持久化 + 增量/全量切换 + 独立 task 复用）→ Task 3/4/5/6 ✓
- ③不做 → 明确排除 ✓

**2. 占位符扫描：** 无 TODO/TBD；阶段三字段名有明确标注「以阶段二验证为准」，非占位符而是有条件默认值。Task 6 标注「可选优化」且有降级说明。

**3. 类型一致性：**
- `ChangeKind`（Removed/Modified）在 Task 3 定义，Task 5 `merge_changes_into_cloud_tree` 使用一致 ✓
- `list_changes` / `list_all_changes` 签名在 Task 3 定义，Task 5/6 调用一致 ✓
- `changes_cursor_file` 在 Task 4 定义，Task 5/6 调用一致 ✓
- `ChangesApi::new(client: Arc<DriveClient>)` 在 Task 3 定义，Task 5 全局单例构造一致 ✓
- `changes_api` 字段在 Task 5 加到 struct，构造参数同步 ✓

**4. 潜在风险点（已标注）：**
- 阶段三字段名依赖阶段二 → 已在每个相关 Task 顶部用 ⚠️ 标注
- `merge_changes_into_cloud_tree` 对「新建文件（无已知 rel_path）」跳过，靠下次全量兜底 → 已注释说明
- Task 6 startCursor 不确定 → 标为可选 + 降级说明
