# 10 · Kuikly 重构迁移指南

> 本文档是 **Tauri 2.x（Rust + Vue3）→ Kotlin + Kuikly** 的逐模块迁移指南。
> Kuikly 基于 KMP（Kotlin Multiplatform），逻辑层 Kotlin 共享，UI 层映射各端原生组件。
> 本文假设目标至少包含 **macOS**（保留双向同步核心能力），可扩展 Android。
>
> 所有精确数值（阈值、键名、状态码、并发数、超时、端口、版本号、字节布局）均来自 `src/` 源码逐行核对，散见 `01`–`09` 各章。本文把它们集中为可执行的迁移清单。

---

## 一、技术栈映射总表

| 层 | 原实现（Rust/Vue） | Kuikly 重构 | 说明 |
|---|---|---|---|
| 语言 | Rust（edition 2021, MSRV 1.77）+ TypeScript | Kotlin | 全 Kotlin（KMP commonMain 共享逻辑） |
| 异步 | tokio（full features）+ async/await | Kotlin 协程（Coroutines + Flow） | `StateFlow`/`SharedFlow` 替代 broadcast channel；见 §二并发映射 |
| HTTP | reqwest 0.12（rustls-tls, stream, multipart） | Ktor Client / OkHttp | 需手动处理华为怪癖（见 §四难点 1）；Upload 客户端须 `redirect = none` |
| 序列化 | serde / serde_json | kotlinx.serialization | 注意驼峰/下划线、String 容忍、`>0x7F → \uXXXX` 转义 |
| 数据库 | rusqlite 0.32（bundled），schemaVersion=5 | Room / SQLDelight | schema v5 迁移逐版本保留（v2→v5）；CAS revision 不可换 synchronized |
| 配置 | JSON 文件 + 迁移 | DataStore（Preferences） | `validate()` 规则必须保留（见阶段 1） |
| 加密 | ChaCha20-Poly1305 + IOPlatformUUID | 平台 Keystore / 保留同等方案 | 见 §五 Token 存储方案 |
| 日志 | tracing + tracing-subscriber + tracing-appender 三层 | Kermit / timber + 自建环形缓冲 | 保留 stdout + 文件 + 缓冲三层；debug 也用 INFO |
| 文件监听 | notify + notify-debouncer-full（FSEvents） | 平台原生（macOS FSEvents via JNI/Native） | **最大重构难点**，见 §四难点 2 |
| 单实例 | tauri-plugin-single-instance | 文件锁 / 端口占用 | 必须保留（防双进程 FSEvents 互相触发 sync cycle） |
| 托盘 | objc2-app-kit NSStatusItem | 平台原生（NSStatusItem） | Kotlin/Native 或 JNI；重建节流 5000ms |
| 自启 | LaunchAgent plist（带 `--hidden`） | 平台原生 | macOS LaunchAgent + launchctl bootstrap/bootout |
| UI | Vue3 + Pinia + Mate 组件库（27 个） | Kuikly 声明式 UI + 组件库 | 重建 27 个 Mate 组件，保持视觉规范 |
| 状态管理 | Pinia stores（5 个 setup store） | ViewModel + StateFlow | `applyState` 旧 revision 拒绝；`loadAll` 两重乱序保护 |
| IPC | Tauri invoke（49 命令） | 直接方法调用（同进程） | 无需序列化桥；49 命令逐条对应 ViewModel 方法 |
| 事件 | Tauri emit（4 事件） | SharedFlow | `sync_state` / `folder_content_changed` / `transfer_update` / `upload_failed` |
| 更新 | tauri-plugin-updater（Ed25519 + `.app.tar.gz` + `.sig`） | Sparkle（macOS）/ 自建 | 签名链路需重建（见 §四难点 7） |

---

## 二、Rust → Kotlin 并发映射（精确表）

| Rust 概念 | Kotlin 对应 | 注意事项 |
|---|---|---|
| `tokio::sync::broadcast::channel` | `SharedFlow`（热流，多订阅者） | 状态发布用；`StateFlow` 用于需最新值场景 |
| `tokio::sync::Mutex` | `kotlinx.coroutines.sync.Mutex` | **跨 suspend 点可持有** |
| `parking_lot::Mutex` | `synchronized` / `ReentrantLock` | **不可跨 suspend 点持有**（同步锁，持锁 await 会阻塞线程） |
| `Arc<T>` | 共享不可变引用（Kotlin 默认） | 可变共享状态须配合锁或 `AtomicXxx` |
| `Lazy<T>` / `once_cell::Lazy` | `by lazy` / `object` 单例 | — |
| `tauri::async_runtime::spawn` | `CoroutineScope.launch` / `viewModelScope.launch` | — |
| 全局 Lazy 单例（`AUTH_SERVICE` / `MOUNT_MANAGER` / `SYNC_ENGINE` 等） | `object` 单例 或 DI（Koin） | — |
| `AtomicBool` / `AtomicU64` | `AtomicBoolean` / `AtomicLong` | CAS revision 用 `AtomicLong` 或 DB 行锁 |
| `Notify::notify_waiters` | `CompletableDeferred` / `Channel` | follower 等待须**先检查 result 再 await**（防 notify 丢失） |

### 关键锁选择原则（源码核对）

原项目区分两种 Mutex 的使用边界，Kotlin 重构必须等效保持：

- **`parking_lot::Mutex`（同步锁，不可跨 await 持有）** 用于：
  - `current`（内存 token：`TokenRefresher.current`）
  - `refresh_flight.active`（Singleflight 活跃 flight 指针）
  - `cloud_tree`（云端树快照）
  - `recently_deleted_paths`（防振荡 HashMap）
  - `path_to_id`（目录创建回填索引）

- **`tokio::sync::Mutex`（异步锁，可跨 await 持有）** 用于：
  - `current_verifier`（PKCE verifier，登录流程跨多步 await）
  - `cancelled`（OAuth 取消标志）
  - `CycleCoordinator.owner`（唯一 owner 锁，串行化同步周期）
  - `StatusAggregator.publication`（状态发布独占锁，跨 DB 读写）

> Kotlin 实现建议：跨 suspend 的临界区一律用 `kotlinx.coroutines.sync.Mutex`；纯内存字段保护可用 `synchronized`，但**绝不在 `synchronized` 块内调用 suspend 函数**。

### 状态发布模式

```
StatusAggregator（单一 publication lock + 单调 revision）
  → SharedFlow<SyncGlobalState>（revision = next_revision.fetch_add(1) + 1）
    → SyncViewModel.collect → StateFlow → UI 自动更新
```

`update_running_transfer` **刻意不递增 state_revision**（迟到回调守卫）：进度回调可能迟到，若它递增 revision，会让并发状态迁移误判为 stale。用 `WHERE state = Running` 限定即可——任务迁出 Running 后迟到回调 affected rows = 0 自动作废。

---

## 三、模块迁移路线（6 阶段，按依赖关系自底向上）

每个模块标注源码文件与精确数值，便于逐一对照实现。

### 阶段 1：基础设施（无外部依赖）

1. **core/config** → `AppConfig` data class + DataStore 持久化 + `validate()` 校验
   - `concurrency ∈ [1, 20]`（默认 6）
   - `pollIntervalSec == 0` 或 `>= 60`（不允许 1–59；默认 60）
   - `debounceSec >= 1`（默认 3）
   - `oauthCallbackPort > 0`（默认 9999）
   - `mountConfigured == true` 时：`mountDir` 非空、绝对路径、排除 `//` / `Home` / `Application Support`
   - `mountDir` 中 `~` 展开为用户家目录（`expanded_mount_dir`）

2. **core/logging** → 三层日志
   - stdout（控制台）
   - 滚动文件 `PetalLink.log`，每日轮转，`cleanup_old_logs` 保留 **30 天**
   - 环形缓冲 `MAX_BUFFER_SIZE = 1000`，newest-first，供设置页查看
   - `EnvFilter` 默认 `info`；**debug 模式也用 INFO**（避免对 17K 文件做 BFS 时产生数万条 FINE 日志）
   - **绝不打印完整 token/secret**（含截断形式也不打）

3. **core/net_guard** → TCP 连通性探测 + Online/Offline StateFlow
   - 探测目标：`driveapis.cloud.huawei.com.cn:443`（华为 Drive API 域名）
   - 探测方式：TCP connect，**3s 超时**
   - 探测间隔：**30s**
   - 状态稳定化：失败→立即转 Offline；**连续 2 次成功才转 Online**（防抖；离线快上线慢）
   - 被动请求失败 `report_request_network_failure()`：最多发布一次 Online→Offline 边沿（重复失败不重复边沿）
   - 代次管理（`ProbeLifecycle`）：`start()` generation `wrapping_add`（跳过 0），`accepts/finish` 拒绝陈旧任务回写（防 shutdown 后旧探测污染状态）
   - checkpoint 未追平不进 planner：`cloud_tree_is_trusted() == false` 时 `cycle.restore(request)`

4. **error** → `AppError` sealed class + 扁平序列化 + 恢复策略元数据
   - `kind`：`Auth` / `Token` / `DriveApi` / `Config` / `QuotaExceeded` / `Generic`
   - `message`（用户可读中文，始终非空）/ `code?` / `status_code?` / `error_code?`
   - 恢复策略元数据：`RequestSemantics`（Read/Write）/ `DriveTransportKind`（Connect/Timeout/ResponseBody/Decode/Request/Other）/ `RetryAfter`（DelaySeconds / AtUnixMs）
   - 错误消息只含用户可读中文，不泄露协议细节、URL、内部错误码给终端用户

5. **data** → Room/SQLDelight schema v5 + migrations + repository
   - 新库直达终态（v5 全部表/列/索引一次性建好，`user_version = 5`）
   - 旧库逐级 `upgrade_vN`：v2（分片上传上下文 server_id/upload_id/resume_offset）→ v3（sync_items.local_size）→ v4（session_url）→ v5（状态机上下文 11 列 + state 编码迁移 + 错误归一化）
   - v5 state 编码迁移：旧 0/1/2→Pending(0)，旧 3→Completed(6)，旧 4→Failed(7)，旧 5→Canceled(8)；无法安全恢复的活动任务标 Failed
   - CAS revision repository（`transition_transfer` 见 §四难点 4）

### 阶段 2：华为 API 客户端（核心，最易踩坑）

6. **drive/ascii_json** → `\uXXXX` 转义函数（**先实现并测试**）
   - 华为 Drive API 服务端 JSON 解析器**不接受 UTF-8 多字节字符**直接出现在 JSON 值中（即使 Content-Type 声明 charset=utf-8）
   - 算法 `escape_non_ascii`：按 char（Unicode scalar）遍历；`code > 0x7F` 时，BMP（`code <= 0xFFFF`）push `\uXXXX`（小写 hex 4 位）；辅助平面（`code > 0xFFFF`）拆 UTF-16 代理对：`v = code - 0x10000`，`high = 0xD800 + (v >> 10)`，`low = 0xDC00 + (v & 0x3FF)`，push 两个 `\uXXXX`
   - `ascii_json_encode<T>` 先 `serde_json::to_string` 再 `escape_non_ascii`
   - **注意**：multipart 上传的 metadata 部分容忍 UTF-8，**不需要转义**；只有 `application/json` 的 Create/Update 需要

7. **drive/client** → Ktor Client + Bearer 注入 + 401 自动刷新重放
   - 普通客户端：`connectTimeout = 15s`，`timeout = 60s`，`poolMaxIdlePerHost = 15`
   - Upload 客户端：`timeout = 120s` + **`redirect(Policy::none)`**（禁用自动重定向，华为 resume 的 308/Location 不应被自动跟随）
   - `execute_with_retry`：第一次发送 → 非 401 走 `ensure_success_response`；401 → `auth.refresher().refresh()` → `build_authed_with_token(新token)` 重放一次（`auth_already_replayed=true`）
   - `build_authed`：URL 含 `oauth2/v3/token` 时**不注入 auth**（否则循环刷新）；否则 `ensure_valid_access_token()`（距过期 < 60s 主动刷新）+ bearer
   - `classify_transport_error`：kind 优先级 `is_connect > is_timeout > is_body > is_decode > is_request > Other`；`request_may_have_reached_server = semantics.is_write() && transport_kind != Connect`
   - `parse_retry_after`：delta-seconds（纯数字）→ `DelaySeconds`；RFC2822 日期 → `AtUnixMs`

8. **auth** → OAuth + PKCE + token 存储 + 用户信息（详见 `07`）
   - PKCE：`code_verifier` = 64 字节 CSPRNG → base64url 去 `=` 填充（约 86 字符）；`code_challenge` = `SHA256(verifier 字节)` → base64url 去填充（约 43 字符）；`code_challenge_method = S256`；`state` = 32 字节 CSPRNG → hex 小写（64 字符）
   - 授权 URL 参数顺序固定：`response_type → client_id → redirect_uri → state → access_type → code_challenge → code_challenge_method → scope`（最后）
   - scope 单独编码：`SCOPES.join(" ").replace(' ', "%20")`，**`/` 不编码**（否则 1101 invalid scope）
   - 授权码换 token：**手工拼 form body，逐字段 `enc()`**（`+` → `%2B`，否则被 form-urlencoded 当空格 → 1101 invalid code）
   - token 存储：`token.bin`，布局 `[魔数 4B "PTL1"][nonce 12B][ChaCha20Poly1305 密文 + 16B tag]`；明文 length-prefixed 小端，**access/refresh/scope 用 u64，token_type 用 u32**（混用，重构陷阱）
   - Singleflight 并发去重：首个调用者为 leader 执行刷新，follower 等待；follower **先检查 result 再 await**；`RefreshLeaderGuard::Drop` 防 follower 永久阻塞；`clear_if_active` 防 ptr 误清新 flight
   - 用户信息三端点并行（`tokio::join!`，任一失败不阻断）：`userinfo`（常 404 静默跳过）/ `getInfo`（`getNickName=1`）/ `getPhone`（**body 可能纯文本或 JSON**，先试 JSON 失败则包装 `{"mobile": text}`）

9. **drive/files_api** → list / get / create / update / delete / search
   - **list**：用 `queryParam='id' in parentFolder` 语法（**不用 `parentFolder` 参数**）；单引号必须存在；根目录用 `'root'`；`pageSize` 1-100（`PRODUCTION_PAGE_SIZE=100`）
   - **list_all**：固定 pageSize=100 循环至 `nextCursor` 空；HashSet 检测 cursor 循环；达 max_pages(1000) 仍 nextCursor → 错误（**绝不返回部分结果**）
   - **get**：`GET /files/{percent_encode(fileId)}?fields=*`
   - **create**：`mimeType` 必填（否则 21004001）；root 省略 `parentFolder`；中文名必须 ASCII 转义（否则 21004002）；**非幂等**——写前 `find_unique_folder_in_parent` 查重，POST 失败后再查重一次
   - **update**：重命名/描述 `PATCH`；移动追加 `addParentFolder={enc(newId)}&removeParentFolder={enc(oldId)}`（**成对 query 参数，不在 body 写 parentFolder**）；成功必须 **200**（`require_official_write_ok` 仅接受 200）+ `File`
   - **delete**：`PATCH {"recycled": true}`（**不用 DELETE**，DELETE 是永久删除）；成功合同 200 + File.id==请求 id + recycled=true（明确布尔）
   - **search**：`queryParam={enc(query)}`；`validate_query_literal` **拒绝含 `'` 或 `\` 的输入**（fail closed）

10. **drive/upload_api** → multipart + resume（详见 `03` §18-22）
    - **小文件（≤20MB）**：`POST /upload/drive/v1/files?uploadType=multipart`；Content-Type **`multipart/related; boundary=hwcloud_{timestamp_micros}`**（不是 form-data）；第 1 部分 `application/json`（metadata，容忍 UTF-8 不转义），第 2 部分 `application/octet-stream`
    - **大文件（>20MB）resume**：
      - init：`POST ?uploadType=resume` + `X-Upload-Content-Length`；**从 Location 响应头提取 session_url**（body 仅 `{"sliceSize":...}`，不含 serverId/uploadId）
      - 分片 PUT：直接用 Location URL；中间响应 **HTTP 308**（正常响应，不重试），body `rangeList`；`parse_confirmed_offset` 严格校验从 0 开始、连续、无重叠、不越界，返回 `end+1`；**只按服务端确认 offset 前进，禁止用 offset+chunkLen 推算**
      - 最终查询：PUT `Content-Range: bytes */{total}` + `Content-Length: 0`；最多 `FINAL_STATUS_MAX_POLLS=5` 次，间隔 `process_time_ms`（clamp 250..3000ms，缺省 3000）
    - 常量：`SMALL_LARGE_THRESHOLD=20MB` / `SAFE_EXISTING_UPDATE_MAX_BYTES=20MB` / `MIN_CHUNK_SIZE=256KB` / `DEFAULT_CHUNK_SIZE=2MB` / `MAX_CHUNK_SIZE=64MB` / `CHUNK_RETRIES=3`
    - **Update 永不降级 Create**：`reject_unsafe_large_update` >20MB 既有文件替换明确拒绝（`restart_required`）

11. **drive/download_api** → 流式 + `.tmp` + Range + 原子写
    - 路径：`tmp_path = dest + ".tmp"`；`resume_metadata_path = dest + ".download-meta.tmp"`
    - `fetch_remote_metadata`：GET `/files/{enc_id}?fields=*`，读响应头 ETag；body 校验 id==file_id
    - `validated_resume_offset`：tmp 不存在→0；stored != current 或无 stable_identity → discard→0；tmp length > size → discard→0；否则返回 tmp length（**只认 .tmp 实际文件长度，不推算**）
    - Range 下载循环（`restarted_from_zero` 标志，**只允许一次安全回退**）：416 + offset>0 + 未重启过 → discard + 重写 metadata + offset=0 + continue；**200→0**（服务端忽略 Range 截断从 0 写）；**206→解析 Content-Range** 校验
    - `verify_and_install`：size 复核 → **sha256 流式校验（1MB buffer）** → **再 fetch_remote_metadata 一次**（防无 ETag 时把两个云端版本混为一次成功） → `tokio::fs::rename(tmp, dest)`（POSIX 原子替换）
    - `cleanup_if_permanent`：仅永久失败清除断点；暂态失败（无 status_code / 401/408/409/425/429/5xx）保留 .tmp 现场供续传

12. **drive/changes_api** → getStartCursor + list
    - `getStartCursor`：华为 `/changes` 强制要求 cursor；校验 `category==drive#startCursor`，`startCursor` 非空（纯数字字符串）
    - `list`：`nextCursor`（翻页）vs `newStartCursor`（末页 checkpoint），**禁止合并**；空的中间页仍继续（nextCursor 非空就翻）；达 max_pages(10000) → 错误
    - 三种删除信号：`deleted==true` **或** `changeType=="trashDone"` **或** `file.recycled==true` → Removed
    - Change 严格解析：`fileId` 非空；`file.id == fileId`（不一致→错误）；Modified 必须有可完整解析 file + **且只能有一个 parentFolder**
    - cursor 无效 → 400；cursor 过期 → 410（`21084100` CURSOR_EXPIRED）；失败保留旧 checkpoint，回退全量

13. **drive/thumbnail_api / about_api** → 缩略图 + 配额
    - 缩略图：`GET /thumbnails/{fileId}?form=content`，**用 raw_http + 手动 bearer**（不走 401 重放），返回二进制
    - 配额：`GET /about?fields=*`（`fields=*` 强制，否则 400）；配额在 `storageQuota` 子对象，且为 **String 类型**需 `tolerant_parse_int`（接受 int/num/String）

### 阶段 3：本地挂载与监听（平台相关，难点）

14. **mount/manager** → 占位符 + scanLocal + Finder 灰标（详见 `04` §9 / `07` §四）
    - **2 个 xattr 键**（inode 方案，详见 `11-基于inode的文件身份识别方案.md`）：
      | 键 | 取值 | 作用 |
      |---|---|---|
      | `com.hwcloud.state` | `placeholder` / `downloaded` | **占位状态唯一权威判据** |
      | `com.apple.FinderInfo` | `buf[9] = 0x02`（label index 7） | Finder 灰标，仅视觉 |
    - ⚠️ inode 方案变更：原 `fileId`/`size`/`freeUpRelativePath` 三键已删除。文件身份改由 **`local_inode_map` 表**（inode→fileId 映射）承担；释放空间恢复路径改由 **`free_up_staging` 表**记录
    - **Finder 灰标直接写 xattr**（读 32 字节 buf → 改 `buf[9]=0x02` → 写回；清则清零，全零则 `remove_xattr`）；**不用 osascript，不用 Finder API**
    - JNI/Native 读写 xattr（`setxattr`/`getxattr`）；inode 读取用 `Files.getAttribute(path, "unix:ino")`（JVM）或 `stat().st_ino`（Native）
    - `create_placeholder_if_needed`：已存在且**无 state xattr** → 视为用户文件，**绝不转换**；创建只写 state + 灰标 + DB 写 inode 映射（`identity.upsertMapping`）
    - `create_placeholder_strict`：严格版（破坏性流程专用），**不做检查直接 `create_new`**，只写 state xattr
    - `backup_modified_placeholder_if_needed`：占位被写入（state=placeholder 但 size>0）→ 改名备份 `.local-<timestamp>` + 清备份 state xattr（副本天然产生新 inode，无需清 fileId）
    - `delete_local`：0 字节**必须是 placeholder 才删**（保护 `.gitkeep` 等用户空文件）

15. **mount/local_watcher** → FSEvents + 3s debounce + **2s warmup**（非 8s）+ 纯事件驱动
    - Kotlin/Native：直接用 Foundation 的 `FSEventStreamCreate`
    - JVM：JNI 调用 FSEvents API
    - 原实现：`notify`（封装 FSEvents）+ `notify-debouncer-full`，`RecursiveMode::Recursive`
    - **必须保留**：3s debounce + **2s warmup**（防历史回放）+ 纯事件驱动（无轮询）
    - 代次（generation）机制防旧任务喂事件

16. **mount/file_hasher** → SHA256 流式
    - 64KB 缓冲；mtime/size 缓存（避免重复哈希）

17. **mount/skip** → `.hwcloud_` 前缀 + legacy + glob
    - `.hwcloud_` 前缀（`INTERNAL_FILE_PREFIX`，全局硬编码过滤）
    - legacy `.hwcloud_placeholder`
    - `.tmp` 后缀（`TMP_SUFFIX`）
    - glob `skipPatterns`（默认 `.DS_Store` / `.tmp` / `~$*` / `.Trash`）；手写 glob→regex 转换（`*` → `.*`，`?` → `.`）

### 阶段 4：同步引擎（核心业务，详见 `06`）

18. **sync/cloud_tree** → BFS 8 并发 + validate_trusted + 原子 checkpoint + 增量
    - `INDEXING_CONCURRENCY = 8`；每轮 `batch_size = min(8, queue.len())`，`join_all` 并发 `api.list_all(parent_id)`
    - `detect_root_folder_id`：统计 parent_folder 高频值，**最高频并列则 fail closed**
    - 失败节点 retries < 2 重试，否则整体 Err（子树缺失不可接受）
    - `validate_trusted`：`complete` + `cursor` 非空 + 每个路径非空 + fileId 非空 + fileId 唯一 + `path_to_id[path] == tree[path].id` 双向一致 + 空路径→root_folder_id
    - 原子写：`.json.tmp`（create+truncate+write+sync_all）→ 旧文件 hard_link `.json.bak` + sync_parent → rename → sync_parent_directory → 成功 remove `.bak`；失败回滚
    - 增量 `apply_changes_to_candidate`：构建 `id_to_path` 反向索引；Removed 删自身 + `prefix = "{path}/"` 子树；Modified 校验 + rename/move 重键 `rekey_candidate_subtree`
    - `INCREMENTAL_FORCED_FULL_THRESHOLD = 300`（连续 300 次增量后强制全量）；增量失败 → `set_cloud_tree_trusted(false)` + 回退全量
    - **严格完整空盘是合法 checkpoint**（`complete=true` 且 tree 空，不触发误重建）

19. **sync/planner** → 3-way diff **24 种决策**（不是 13 种）
    - `SyncActionType` 10 种：`Upload` / `CreatePlaceholder` / `Download` / `DeleteFromCloud` / `DeleteFromLocal` / `CreateConflictCopy` / `Skip` / `CreateFolder` / `MoveInCloud` / `BackupBeforeCloudDelete`
    - `is_local_changed`：mtime 或 size 与 DB 不同（v3：mtime 精度不足兜底）
    - `is_cloud_changed`：**仅比较 editedTime**（云端时间为权威基准，不比 size）
    - 不可信守卫：`!cloud_tree_trusted && (DeleteFromLocal|DeleteFromCloud)` → 丢弃并 warn（**只抑制删除**）
    - pending 收敛：`PENDING_FILE_PREFIX = "pending:"`；第 6 项 Skip 携 cloud_file 收敛；第 12 项 FAILED pending 不自动重试
    - 启动恢复守卫：`is_startup_resume` 时 cloud_tree 不可信的删除跳过待复核

20. **sync/conflict** → 60s 容忍 + 副本去重 + 目录保护 + 目录救援补建
    - **60s 容忍**：`delta.num_seconds > 60` 本地胜（仅当本地比云端晚 > 60s）；否则云端胜
    - 副本去重 `dedupe_copy_path`：格式 `{stem} ({side_label} {YYYY-MM-DD HH-mm-ss}){ext}`，序号 0..1000；时间戳取自**败方**；冒号替换为 `-`（文件系统安全）
    - 目录保护 `preserve_dirs_with_pending_backups`：DeleteFromLocal 目录路径是 BackupBeforeCloudDelete 路径祖先 → 移除该 DeleteFromLocal
    - 目录救援补建 `add_rescue_folder_recreations`：扫描创建动作的祖先前缀，按深度升序插入 `CreateFolder(cloud_file=None)`

21. **sync/stability** → 三段式 + lsof（详见 `06` §7）
    - `MIN_MTIME_AGE_SECS = 5` / `SIZE_STABLE_WINDOW_SECS = 3` / `EDITING_THRESHOLD_SECS = 300`
    - 三段：mtime_age < 5s → Unstable；size1，sleep(3s)，size2 不等 → 检查编辑阈值；lsof busy → sleep(1s) 双重检查 → 仍 busy → 检查编辑阈值
    - lsof：`lsof -nP -F pc <path>`，解析 p 行（pid）、c 行（command）
    - **白名单 10 个只读系统进程**：`mds, mdworker_shared, mdimport, mdflagworker, QuickLookSatellite, qlmanage, corespotlightd, secd, bird, CoreServicesUIAgent`
    - 持续编辑 > 5min 标记 `Editing`（上报 `SyncGlobalState.editing`，前端显示"用户编辑中"暂停自动同步）

22. **sync/executor** → 并发 6 + 目录优先两阶段 + 稳定性接入 + 冲突解决
    - `Semaphore::new(concurrency.max(1))`（默认 6，可配 1-20）；`buffer_unordered(concurrency).collect()`
    - 每个动作拿 slot 后必须通过 `begin_action_activity` 门（engine shutdown 后返回 deferred=true，不记 FAILED）
    - **两阶段目录优先**：阶段 1 收集 `CreateFolder && cloud_file.is_none()`，按路径深度（`/` 计数）升序**顺序执行**，每个成功后立即 `apply_results` + 回填 `path_to_id`；阶段 2 `fill_parent_file_ids` 回填其余动作 parent_file_id，然后并发执行
    - 稳定性检查：重试窗口 `[0, 2, 3, 5]` 秒（共 4 次），仅 Upload/Create/Update 前
    - 冲突 `do_conflict`：Cloud 胜 rename→download+mark_downloaded+清灰标；Local 胜先 download 云端旧版到 copy_path 再 upload_update 覆盖
    - 上传成功后回写：`edited_time.is_none()` → `files_api.get(id)` 补全；**inode 方案**：成功后在 DB 事务内 `identity.upsertMapping`（确定性记账，非 xattr 补写）
    - 结算 `settle_success`（原子事务）：上传用 `running.source_mtime/source_size`（任务持久化快照，**不是当前 stat**），下载用当前 stat；清 pending 孤儿行；Update 额外清旧路径

23. **sync/task_runner** → 九态状态机 + CAS + 断点续传 + 远端核验（详见 `06` §4）
    - 九态：Pending(0) / Running(1) / WaitingForNetwork(2) / BackingOff(3) / VerifyingRemote(4) / RestartRequired(5) / Completed(6) / Failed(7) / Canceled(8)
    - `can_transition` 用 `when` 严格校验；**Completed 与 Canceled 是纯终态，无任何出边**；非法迁移抛 `IllegalTransition`
    - CAS revision：`UPDATE ... SET state_revision=state_revision+1 WHERE id=? AND state_revision=?`；`changed != 1` 报 `StaleRevision`
    - `ColumnPatch<T>` 三态：Keep(0)/Set(1)/Clear(2)
    - `transition_transfer_clearing_upload_session`：转换后**同事务**原子失效 server_id/upload_id
    - **数据安全关键规则**：
      - 上传恢复：进程重启遇到 Running 上传 **绝不推算 offset**，改迁 `VerifyingRemote`，由 `resume_verifying` → `verify_remote` GET/list_all 核验；带 session_url 的续传只认服务端 rangeList 确认的 offset
      - 下载恢复：`.tmp` + 版本 sidecar；`durable_offset = metadata(&.tmp).len().min(total_size)`（**只认 .tmp 实际文件长度**）
      - 写操作结算（禁止盲目重放）：`validate_success_outcome`（上传须 cloud_file.id 非空 + name 匹配 + edited_time 有值 + size 匹配；下载须 `metadata.len() == total_size`）；`verify_remote`（Create 404→Ambiguous 禁止重复创建；Update GET 比对 edited_time）；RestartRequired 含 remote_result_id 自动提升 VerifyingRemote
    - **启动恢复固定 8 步顺序**：reset_stale_statuses → load_or_refresh_cloud_tree → promote_ambiguous_restarts → recover_verified_remote_path_changes → purge_deleted_tombstones_if_trusted → STARTUP recover_interrupted_transfers + commit_recovery_checkpoint → ONLINE_RECOVERY resume_verifying → resume_waiting → resume_due_backoff（严格此顺序）→ run_sync_cycle_inner
    - retry_policy：error_kind 12 种；退避 `(1 << attempt).coerceAtMost(300)` 秒 + jitter，上限 300s；`MAX_AUTOMATIC_ATTEMPTS = 5`；退避序列 1s/2s/4s/8s/16s

24. **sync/engine** → CycleCoordinator + ActivityTracker + 防振荡 + StatusAggregator
    - `CycleCoordinator`：单 owner 合并请求；**CycleRequest 位集 7 位**（`LOCAL_RESCAN | CLOUD_INCREMENTAL | CLOUD_FULL | ONLINE_RECOVERY | STARTUP | RETRY | REPLAN`）；`pending |= request`，序列号 `wrapping_add(1).max(1)`；`take_pending_with_sequence` 取走 pending；`restore` 未执行位恢复（sticky）；失败历史超 128 条淘汰
    - `ActivityTracker`：路径租约；`accepting` 标志，`close()` 后拒绝新活动；`wait_idle()` 等待所有已登记活动释放；`is_indexing` 期间拒绝 watcher/手动 cycle（`folder_syncing || syncing || shutdown`）
    - 防振荡 `recently_deleted_paths`：`HashMap<String, i64>`；写入时机 action 成功且类型为 DeleteFromCloud/DeleteFromLocal/BackupBeforeCloudDelete；TTL 清理 `retain(|_, ts| ts > now_ms - 300_000)`（**5 分钟**）；`filter_anti_oscillation` 丢弃近期删除路径回摆动作，**但保留 DeleteFromCloud**
    - `StatusAggregator`：单一 publication lock + 单调 revision（`next_revision.fetch_add(1) + 1`）；`SyncGlobalState` 聚合 SQL 单次 query_row 7 个子查询

### 阶段 5：平台集成

25. **platform/tray** → NSStatusItem + 菜单
    - 菜单项：version / show_window / 传输段 / quit
    - 重建节流 **5000ms**（避免频繁重建）；`transfer_signature` 防闪烁

26. **platform/activation** → `--hidden` 检测 + activationPolicy 切换
    - `activationPolicy`：`regular=0`（有 Dock 图标）/ `accessory=1`（仅菜单栏）
    - **swizzle `NSApplication terminate:`**（objc2 `set_implementation`）：拦截 Dock/Cmd+Q
    - 检测 Apple Event 区分系统关机：`kCoreEventClass = 0x61657674`（'aevt'）/ `kAEQuitApplication = 0x71756974`（'quit'）→ 放行真正退出
    - accessory 模式窗口恢复须切 regular（否则按钮点不动）

27. **platform/launch_at_login** → LaunchAgent plist
    - plist 带 `--hidden` 参数（开机只显示菜单栏图标）
    - `launchctl bootstrap` / `bootout`
    - 与 Login Items 去重（若 LaunchAgent 已启用，移除 Login Items 重复项）

28. **platform/shutdown** → flush + 哨兵
    - `shutdown()`：等待旧同步周期完成 → 等待已提交任务完成结算 → 若 `is_indexing`（BFS 进行中）→ `mark_cache_incomplete_if_exists` → drop_runtime（释放 FSEvents）
    - **3.2s 超时兜底**

29. **单实例守护** → 文件锁 / 端口占用
    - 第二进程启动 exit(0) + 聚焦已运行实例窗口
    - 带 `--hidden`（LaunchAgent 重复触发）不显示窗口；否则 show + focus + `set_regular()`（accessory 切回 regular）
    - **为什么必须**：防双进程各自创建 FSEvents watcher 监听同一挂载目录 → 互相触发 sync cycle → 基于 stale cloud_tree 误判"本地新建"疯狂上传

### 阶段 6：UI 层（Kuikly）

30. **design token** → 主题系统（详见 `09`）
    - 主色 `#0052D9`（brand）/ hover `#366EF4` / active `#003CAB` / light `#D9E1FF` / lighter `#F2F3FF`
    - 功能色：success `#2BA471` / warning `#E37318` / error `#D54941`
    - 14 级灰阶；语义别名（浅色/深色）；macOS 窗口专用色
    - 间距 4-grid（xs=4 / sm=8 / md=12 / lg=16 / xl=24 / xxl=32）；圆角 sm=3 / md=6 / lg=9
    - 深色模式跟随系统（`@media (prefers-color-scheme: dark)`）

31. **Mate 组件库** → 重建 27 个（详见 `08` §二）
    - 优先级：高频（Button / TextField / Tag / Dialog / Toast / Icon / Progress）→ 布局（Scaffold / SectionHeader / NavItem）→ 特化（PopupMenu 视口钳制 / Stepper / Switch）
    - `useDialog`：`confirmDialog` Promise + resolver 命令式 await
    - `useToast`：单条语义
    - 保持视觉规范（圆角 3/6/9px、字号阶梯、阴影层级、`prefers-reduced-motion` 降级）

32. **ViewModel** → auth / sync / transfer / fileBrowser / updater
    - **sync `applyState`**：缺字段/默认对象/旧 revision 均不改变 UI；同一 revision 重复投递只允许幂等赋值，**不能重复触发目录刷新**；新 revision 才 `sidebarRefresh++`
    - **transfer `loadAll` 两重乱序保护**：requestId 递增（丢弃乱序响应）+ revision CAS（同一 task 旧 state_revision 不回写）
    - **updater `waitForTransfers`**：轮询等队列空闲，最多 5min，每 2s 一次；节流统一收敛 `throttledCheck()`（启动首次强制 / 定时 60min / 焦点 10min / 手动不节流）

33. **页面** → Login / Main / Settings / LogViewer
    - **FileListView**（938 行，最大文件）：6 列拖拽 + 右键异步 `canFreeUp`
    - **Sidebar**：递归目录树，并发安全路径联动
    - 路由：`initial + loading → Splash`；`loggedIn + currentPage=settings → SettingsPage`；`logs → LogViewerPage`；`main → MainPage`；其他 → LoginPage

---

## 四、关键迁移难点与对策（7 个）

### 难点 1：华为 API 怪癖（最高优先级）

**必须 100% 复刻** `03-华为Drive-API接口规范.md` 的 18 条踩坑清单。核心：

- **先写 `asciiJsonEncode` 单元测试**（覆盖中文、emoji、代理对：BMP 用 `\uXXXX`，辅助平面用 UTF-16 代理对拆两个 `\uXXXX`）
- HTTP 客户端层统一注入 Bearer，但 **token 端点除外**（URL 含 `oauth2/v3/token` 跳过，否则循环刷新）
- **scope 拼接单独函数**：`SCOPES.join(" ").replace(' ', "%20")`，**`/` 不编码**（否则 1101 invalid scope）
- **授权码换 token 手工拼 form body**：逐字段 `enc()`，`+` → `%2B`（不用 `.form()`）
- **multipart 用 `multipart/related`**（非 form-data），boundary `hwcloud_{timestamp_micros}`
- **308 当正常响应**，不重试，不重定向；Upload 客户端 `redirect(Policy::none)`
- **nextCursor（翻页）vs newStartCursor（末页 checkpoint）禁止合并**；空中间页继续
- 配额字段 String 类型，`tolerant_parse_int` 接受 int/num/String
- 软删除成功合同 200 + File.id==请求 id + recycled=true
- 严格 schema 校验：`parse_file_list_page`（category/files 必须数组）/ `parse_drive_file_strict`（id/fileName/mimeType 非空）/ `require_official_write_ok`（仅 200）/ `single_parent`（只支持一个非空 parent）

### 难点 2：文件监听（macOS FSEvents）

- **Kotlin/Native**：直接用 Foundation 的 `FSEventStreamCreate`
- **JVM**：JNI 调用 FSEvents API
- **必须保留**：3s debounce + **2s warmup（非 8s）**（防历史回放）+ 纯事件驱动（无轮询）
- 代次（generation）机制防旧任务喂事件
- 原实现：`notify`（封装 FSEvents）+ `notify-debouncer-full`，`RecursiveMode::Recursive`

> Android 端用 `FileObserver`，但本项目核心是 macOS，Android 可后续扩展。

### 难点 3：占位符 xattr 模型（macOS 专属，2 个键 + inode 映射）

- **2 个 xattr 键**（inode 方案，详见 `11-基于inode的文件身份识别方案.md`）：`com.hwcloud.state` / `com.apple.FinderInfo`
- **文件身份由 `local_inode_map` 表承担**（inode→fileId 映射），不写 fileId xattr
- **`com.hwcloud.state` 是占位状态唯一权威判据**（`placeholder` / `downloaded`）
- JNI/Native 读写 xattr（`setxattr`/`getxattr`）；inode 读取 `stat().st_ino`
- **Finder 灰标直接写 `com.apple.FinderInfo`**（`buf[9]=0x02`，label index 7），**非 osascript，非 Finder API**
- **绝对原则**：无 xattr 文件视为用户文件，**绝不转换**；0 字节非占位**不删**（`.gitkeep`）

### 难点 4：九态传输状态机

- 用 Kotlin sealed class / enum 实现 `TransferState`
- `can_transition` 用 `when` 表达式严格校验；**Completed(6) 与 Canceled(8) 是纯终态，无任何出边**
- CAS revision 用 `AtomicLong` 或数据库行锁（`WHERE id=? AND state_revision=? +1`）
- **数据安全规则不可妥协**：
  - 上传恢复迁 `VerifyingRemote`，**不推算 offset**
  - 下载只认 `.tmp` 实际长度
  - 写操作 `verify_remote` 核验（GET/list_all）
  - 启动恢复固定 8 步顺序

### 难点 5：lsof 稳定性检查

- macOS：`Runtime.exec("lsof -nP -F pc <path>")` 或 JNI 调用 `libproc`
- **白名单 10 个只读系统进程**：`mds, mdworker_shared, mdimport, mdflagworker, QuickLookSatellite, qlmanage, corespotlightd, secd, bird, CoreServicesUIAgent`
- 双重检查 1s（busy → sleep(1s) → 再查）
- Android：用 `FileLock` 或跳过此检查

### 难点 6：单实例守护 + activationPolicy

- **swizzle `NSApplication terminate:`**（objc2 `set_implementation`）
- 检测 Apple Event 区分系统关机：`kCoreEventClass = 0x61657674`（'aevt'）/ `kAEQuitApplication = 0x71756974`（'quit'）→ 放行
- accessory 模式窗口恢复须切 regular（否则按钮点不动）
- 关窗/退出拦截（CloseRequested/ExitRequested → prevent + hide + accessory）；仅托盘"退出 PetalLink"置 `mark_real_quit()` 真正退出

### 难点 7：应用更新

- 原方案：Tauri Updater（Ed25519 + `.app.tar.gz` + `.sig`）
- macOS 用 **Sparkle**（行业标准，支持 Ed25519 签名）
- 需重建签名密钥对和更新清单格式

---

## 五、Token 存储方案选择

| 平台 | 推荐方案 | 理由 |
|---|---|---|
| macOS（保留原方案） | ChaCha20-Poly1305 + IOPlatformUUID | 跨机器保护 + 无 Keychain 签名依赖；与 Rust 行为完全一致 |
| macOS（简化） | macOS Keychain | 系统级安全，但需处理 dev/release bundle id 隔离 |
| Android | Android Keystore + `EncryptedFile`（Jetpack Security） | 平台原生，依赖硬件密钥 |

原方案字节布局（保留方案须精确复刻）：

```
文件：[魔数 4B "PTL1"][nonce 12B 随机][ChaCha20Poly1305 密文 + 16B Poly1305 tag]
明文：[u64 access_len][access_bytes]
      [u64 refresh_len][refresh_bytes]
      [i64 expires_at]                    // 毫秒
      [u32 token_type_len][token_type_bytes]   // ⚠️ u32 不是 u64
      [u8 scope_present]
        若 1: [u64 scope_len][scope_bytes]
```

> **重构陷阱**：access/refresh/scope 长度前缀用 u64，token_type 用 u32（混用）。`expires_at` 是 i64 毫秒（带符号）。

密钥派生：`machine_uuid()` 执行 `ioreg -d2 -c IOPlatformExpertDevice` 解析 `IOPlatformUUID`；`derive_key(uuid) = Sha256(uuid.as_bytes())`（**不加 salt，不用慢哈希**）。

无论哪种方案，都必须保证：
- token 不日志输出（含截断形式也不打）
- 退出登录彻底清理（token.bin + 内存 + DB + 缓存 + config 挂载字段重置，其余设置保留）
- 跨机器/重装系统后视为未登录
- save 写 `token.bin.tmp` → `set_permissions(0o600)` → rename（原子替换）；clear 文件不存在视为成功（幂等）

---

## 六、测试迁移策略

原项目 23 个 Rust 测试文件（根目录 `tests/`，按领域 `*_test.rs` 命名）+ 前端 Vitest 测试。

| 原测试 | Kotlin 对应 | 验证内容 |
|---|---|---|
| `drive_ascii_json_test` | commonTest | 中文转义**含代理对**（BMP `\uXXXX` + 辅助平面两个 `\uXXXX`） |
| `oauth_flow_test` | commonTest | PKCE（verifier 64 字节 base64url 约 86 字符 / challenge SHA256 / state 32 字节 hex 64 字符）+ 授权码 `+`→%2B 编码 |
| `drive_api_test` / `drive_models_test` / `drive_changes_test` / `drive_client_error_test` / `drive_download_test` | commonTest | API 协议边界（nextCursor vs newStartCursor / 三种删除信号 / require_official_write_ok 仅 200 / single_parent） |
| `error_contract_test` | commonTest | AppError 扁平序列化 + 恢复策略元数据（RequestSemantics/DriveTransportKind/RetryAfter） |
| `sync_engine_test`（含 planner） | commonTest | **24 种决策**（不是 13 种）+ 不可信守卫 + pending 收敛 |
| `sync_transfer_state_test` | commonTest | 九态状态机 + `can_transition` 矩阵 + 枚举数值稳定 |
| `sync_task_recovery_test` | commonTest | VerifyingRemote 远端核验恢复 + 路径许可 + 8 步启动顺序 |
| `sync_retry_policy_test` | commonTest | 错误分类决策树 + 退避算法（2^attempt 上限 300s） |
| `sync_cloud_tree_test` | commonTest | validate_trusted + 增量 rekey_candidate_subtree |
| `sync_conflict_test` | commonTest | 60s 容忍 + 副本去重（序号 0..1000，时间戳取败方） |
| `sync_path_recovery_test` | commonTest | 路径恢复（收敛远端改名） |
| `sync_state_store_test` / `sync_state_test` | commonTest | CAS revision + ColumnPatch 三态 |
| `core_config_test` / `core_logging_test` / `core_paths_test` | commonTest | validate 规则 / 三层日志 / safe_join_under 防穿越 |
| `upload_tester`（真实云端 `#[ignore]`） | `@Ignored` 集成测试 | 真实云端，环境变量显式启用 |

| 原测试基建 | Kotlin 对应 |
|---|---|
| `wiremock` HTTP mock | Ktor MockEngine / MockWebServer |
| Vitest 前端测试 | Kotlin UI 测试 |

**重点测试必须迁移**：`ascii_json` / `oauth_flow` / `sync_planner` / `transfer_state` / `task_recovery` / `retry_policy` / `cloud_tree` / `conflict`。

**`DELETE_TRACE_ERROR_PREFIX` 前后端完全一致**（`"TRACE_FAILED:"`，有 contract test）：前端据此区分"文件未删"与"文件已删但记录未写入"。改动任一侧必须同步另一侧。

---

## 七、建议的工程结构（KMP，对齐实际 `src/` 模块划分）

```
petal-link-kuikly/
├── settings.gradle.kts
├── build.gradle.kts
├── gradle.properties
├── shared/                          # KMP 共享模块（核心业务）
│   ├── build.gradle.kts
│   └── src/
│       ├── commonMain/              # 跨平台共享（对齐 src/ 纯逻辑模块）
│       │   └── kotlin/io/github/yuanbaobaao/petallink/
│       │       ├── auth/            # OAuth + PKCE + token（src/auth/ 8 文件）
│       │       ├── core/            # config / logging / net_guard / cache_paths / paths（src/core/ 8 文件）
│       │       ├── drive/           # 华为 API 客户端（src/drive/ 16 文件）
│       │       ├── sync/            # 同步引擎纯逻辑部分（src/sync/ 35 文件）
│       │       ├── data/            # 数据模型 + repository 接口（src/data/ 接口）
│       │       ├── error/           # AppError（src/error.rs）
│       │       └── ui/              # Kuikly 声明式 UI + ViewModel + 主题
│       ├── macosMain/               # macOS 平台实现（对齐 src/ 平台相关模块）
│       │   └── kotlin/.../
│       │       ├── platform/        # tray / activation / launch_at_login / shutdown（src/platform/ 5 文件）
│       │       ├── mount/           # FSEvents + xattr + manager（src/mount/ 5 文件）
│       │       └── data/            # Room/SQLDelight macOS 实现（src/data/ 实现）
│       └── androidMain/             # Android（后续扩展）
├── macosApp/                        # macOS 应用入口
└── docs/                            # 本文档
```

> `expect/actual` 模式处理平台差异（文件监听、xattr、托盘、加密存储、lsof、LaunchAgent 等）。
> commonMain 承载所有纯逻辑（可单元测试），macosMain 承载所有平台相关实现。

---

## 八、迁移检查清单（精确校验点）

重构完成前，逐项确认（每项都是源码提取的精确值）：

### 华为 API
- [ ] 18 条踩坑全复刻：scope **`/` 不编码** / 授权码 **`+`→%2B** / 中文 `\uXXXX` **含代理对**（辅助平面拆两个）/ **multipart/related**（非 form-data）/ 308 rangeList 连续性 / nextCursor vs newStartCursor / 配额 String 容忍
- [ ] token 端点不注入 Bearer（URL 含 `oauth2/v3/token` 跳过）
- [ ] Upload 客户端 `redirect = none` + timeout 120s
- [ ] `require_official_write_ok` 仅接受 200 + File
- [ ] 删除软删除 PATCH `{"recycled":true}`，不用 DELETE

### OAuth
- [ ] PKCE：verifier **64 字节 base64url 约 86 字符**（去 `=` 填充）/ challenge **SHA256** base64url / state **32 字节 hex 64 字符**
- [ ] 127.0.0.1:9999 loopback（仅 IPv4，绝不 0.0.0.0）
- [ ] 授权 URL 参数顺序固定，scope 最后
- [ ] token 刷新 Singleflight 并发去重（follower 先检查 result 再 await；`RefreshLeaderGuard::Drop` 防永久阻塞）
- [ ] 刷新响应可能不含新 refresh_token → 沿用旧的

### 数据安全
- [ ] 写操作核验 **200+File** 后结算 / 响应丢失 GET 核验 / 禁止盲目重放
- [ ] 上传恢复 **VerifyingRemote 不推算 offset**（只按服务端 rangeList 确认 offset 前进）
- [ ] 下载只认 `.tmp` 实际长度（`durable_offset = metadata(&.tmp).len().min(total_size)`）
- [ ] `verify_remote`：Create 404→Ambiguous（禁止重复创建）；Update 比对 edited_time
- [ ] RestartRequired 含 remote_result_id 自动提升 VerifyingRemote

### 九态状态机
- [ ] `can_transition` 严格校验（Completed/Canceled 纯终态无出边）+ CAS revision（`WHERE state_revision=? +1`）
- [ ] `ColumnPatch` 三态 Keep(0)/Set(1)/Clear(2)
- [ ] `update_running_transfer` **不递增 state_revision**（迟到回调守卫）
- [ ] `transition_transfer_clearing_upload_session` 同事务失效 server_id/upload_id

### 断点续传
- [ ] 上传只按服务端 rangeList 确认 offset 前进（`parse_confirmed_offset` 返回 `end+1`）
- [ ] 下载 Range + 版本核验 + 416 回退（只允许一次）+ sha256 流式校验（1MB buffer）+ **二次 fetch_remote_metadata**
- [ ] verify_and_install：POSIX rename 原子替换

### 占位符与文件身份（inode 方案，详见 `11-基于inode的文件身份识别方案.md`）
- [ ] **2 个 xattr 键**（非 5 个）：state + FinderInfo（fileId/size/freeUpRelativePath 三键已删除）
- [ ] **`local_inode_map` 表**（schemaVersion=6）：inode→fileId 映射，身份识别核心
- [ ] **`free_up_staging` 表**：释放空间事务恢复（替代 `XATTR_FREE_UP_RELATIVE_PATH`）
- [ ] **`identity` 模块**：lookupByInode / upsertMapping / purgeMissing
- [ ] **detect_moves**（替代 detect_renames）：基于 inode 配对，约 40 行，无复制消歧
- [ ] 下载覆盖后 `upsertMapping` 更新 inode 映射（确定性记账，非 xattr 补写）
- [ ] `com.hwcloud.state` 唯一权威判据
- [ ] `com.apple.FinderInfo` `buf[9]=0x02` 灰标（直接写 xattr，非 osascript）
- [ ] 无 state xattr 视为用户文件，**绝不转换**
- [ ] 0 字节非占位**不删**（`.gitkeep`）

### 文件监听
- [ ] 3s debounce + **2s warmup（非 8s）** + 纯事件驱动 + 代次机制

### 稳定性检查
- [ ] mtime > 5s + size 稳定 3s + lsof（白名单 **10 进程**双重检查 1s）

### 冲突
- [ ] 60s 容忍（`delta.seconds > 60` 本地胜）
- [ ] 副本去重（序号 0..1000，时间戳取败方，冒号替换为 `-`）
- [ ] 目录保护 + 目录救援补建

### 防振荡
- [ ] recentlyDeletedPaths **5 分钟 TTL** retain，保留 DeleteFromCloud

### 网络守卫
- [ ] TCP 探测华为域名 443，30s 间隔 3s 超时
- [ ] **连续 2 次成功才 Online**（防抖）+ 代次管理
- [ ] 被动请求失败 Offline 边沿（最多一次）
- [ ] 恢复固定顺序：cloud catch-up → VerifyingRemote → planner

### 内部文件隔离
- [ ] `.hwcloud_` 前缀全局硬编码过滤 + legacy `.hwcloud_placeholder` + `.tmp`

### 单实例 + activationPolicy
- [ ] swizzle `NSApplication terminate:` / Apple Event 区分系统关机（`kCoreEventClass=0x61657674` / `kAEQuitApplication=0x71756974`）
- [ ] accessory 切 regular

### 后台运行
- [ ] 关窗/退出拦截 + accessory
- [ ] 仅托盘"退出"真退出（`mark_real_quit`）

### 释放空间 TOCTOU
- [ ] 13 步（双重本地/DB 复核 + 远端核验夹中间 + 原子 staging + CAS 回滚）
- [ ] `recover_interrupted_free_up`：扫描 `.hwcloud_freeup-` 暂存，读 freeUpRelativePath，已提交删除暂存/未提交恢复

### 设计系统
- [ ] 主色 `#0052D9` + 完整 token + **27 组件** + 深色模式

### 日志
- [ ] 三层（stdout + 文件每日轮转保留 **30 天** + 缓冲 **1000** newest-first）
- [ ] 不打印 token/secret（含截断）
- [ ] debug 也用 INFO（避免 17K 文件 BFS 数万条 FINE 日志）

### 跨语言合同
- [ ] `DELETE_TRACE_ERROR_PREFIX` 前后端完全一致（`"TRACE_FAILED:"`，有 contract test）

### 退避算法
- [ ] `2^attempt` 秒上限 **300s** 加 jitter / `MAX_AUTOMATIC_ATTEMPTS = 5`
- [ ] 退避序列 1s/2s/4s/8s/16s

### 增量阈值
- [ ] `INCREMENTAL_FORCED_FULL_THRESHOLD = 300`（连续 300 次增量后强制全量）
