# 07 · 安全、OAuth 与占位符模型

> 本章覆盖完整登录链路（OAuth 2.0 + PKCE）、Token 自动刷新与并发去重（Singleflight）、机器码绑定的加密存储（token.bin）、Client 凭据注入、用户信息聚合、macOS 占位符模型、退出登录清理与日志安全。
> **协议字段一律保留官方大小写**（如 `access_token`、`code_verifier`、`grant_type`）。代码示例为 Kotlin 等价示意，非 Rust 原文。

---

## 一、OAuth 2.0 + PKCE 登录流程

整体时序：生成 PKCE/state → 启动本地回调服务器 → 构造授权 URL 并打开浏览器 → 用户授权 → 回调收到 `code` → 校验 state → 换 Token → 持久化。

### 1.1 PKCE 生成（精确参数）

| 参数 | 生成方式 | 编码 | 长度 |
|---|---|---|---|
| `code_verifier` | 64 字节 CSPRNG 随机数 | base64url，`URL_SAFE_NO_PADDING`（**去除 `=` 填充**） | 约 86 字符（RFC 6749 限定 43-128） |
| `code_challenge` | `SHA256(code_verifier 的字节)` | base64url，无填充 | 约 43 字符 |
| `code_challenge_method` | 固定字面量 `S256` | — | — |
| `state` | 32 字节 CSPRNG 随机数 | hex（小写） | 64 字符（防 CSRF） |

> 关键：`code_challenge` 哈希的是 verifier 的**原始字节**，不是 base64url 字符串再编码。base64url 一律去 `=` 填充。

Kotlin 等价示意：

```kotlin
data class PkcePair(val codeVerifier: String, val codeChallenge: String)

fun generatePkce(): PkcePair {
    // 1) verifier：64 字节随机 → base64url 去填充
    val verifierBytes = ByteArray(64).also { SecureRandom().nextBytes(it) }
    val verifier = Base64.getUrlEncoder().withoutPadding().encodeToString(verifierBytes)
    // 2) challenge：SHA256(verifier 字节) → base64url 去填充
    val digest = MessageDigest.getInstance("SHA-256").digest(verifier.toByteArray(Charsets.US_ASCII))
    val challenge = Base64.getUrlEncoder().withoutPadding().encodeToString(digest)
    return PkcePair(verifier, challenge)
}

fun generateState(): String {
    // 32 字节随机 → hex → 64 字符
    val bytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
    return HexFormat.of().formatHex(bytes)   // 小写 hex
}
```

### 1.2 授权 URL 构造（参数顺序固定）

基准 URL：`https://oauth-login.cloud.huawei.com/oauth2/v3/authorize`

```
https://oauth-login.cloud.huawei.com/oauth2/v3/authorize?
  response_type=code
  &client_id=118065481
  &redirect_uri=http://127.0.0.1:9999/oauth/callback
  &state={state}
  &access_type=offline
  &code_challenge={challenge}
  &code_challenge_method=S256
  &scope=openid%20profile%20https://www.huawei.com/auth/drive
```

**参数顺序固定**（原实现按此顺序手工拼接，便于排查与 AGC 后台审计）：

```
response_type → client_id → redirect_uri → state
            → access_type → code_challenge → code_challenge_method → scope（最后）
```

**编码规则（重点，错一处即失败）**：

- **scope 单独编码**：先把 `SCOPES` 列表用空格 `join(" ")`，再 `.replace(' ', "%20")`。**`/` 不编码**（保留 `https://www.huawei.com/auth/drive` 中的斜杠）。若把 `/` 也编码成 `%2F`，AGC 会返回 `1101 invalid scope`。
- **其余参数用 `enc()`**：等价于 Dart 的 `Uri.encodeComponent`——RFC 3986 unreserved 集合 `A-Za-z0-9-_.~` 不编码，其余全部 percent-encode。注意 **`+` → `%2B`**（不是空格）。
- `redirect_uri` 中的 `:` `/` `.` 虽在 encodeComponent 下会被编码，但原实现对 redirect_uri 也走 `enc()`（与官方一致，AGC 能正确解码）。

scope 选择说明：用 `drive`（全盘访问），**不能用** `drive.file`（后者仅限本应用创建/打开的文件，网页或其他客户端上传的文件不可见）。必须在 AGC 后台开通 `drive` scope。

### 1.3 本地回调服务器（`oauth_server.rs`）

| 项 | 实现 |
|---|---|
| 监听地址 | `TcpListener::bind("127.0.0.1:port")`，**仅 loopback IPv4**，绝不监听 `0.0.0.0` |
| 默认端口 | `9999`（`oauthCallbackPort`） |
| 回调路径 | `/oauth/callback` |
| 结果通道 | `oneshot::channel` 传回调结果，`watch::channel` 作停止信号 |
| 生命周期 | **单次使用**：`accept` 一笔请求后立即 `break` |
| 超时 | `wait_for_callback` 外层 `timeout(300s)` |

`wait_for_callback` 三条返回路径，**均先 `stop()` 再返回**：

1. 收到回调结果（`oneshot` 收到值）→ 返回 `Ok`
2. 用户取消（`cancelled` 标志置位）→ 返回取消错误
3. 300s 超时 → 返回超时错误

`handle_request` 流程：

- 读取最多 **4096 字节**请求
- 取请求行的 path
- 校验 `path.starts_with("/oauth/callback")`，否则忽略
- 提取 query 串交给 `parse_query`

`parse_query` 编码处理（重点）：**先把 `+` 替换为空格**（form-urlencoded 语义，浏览器回跳的 query 可能含 `+`），再 `percent_decode`。顺序不能反。

响应 HTML：根据是否拿到 `code` 返回成功页或失败页，响应头 **`Connection: close`**（确保浏览器立即关闭连接，回调页可正常展示）。

### 1.4 授权码换 Token（精确编码，重点）

⚠️ **关键**：`authorization_code` 含 `+ / =` 特殊字符。若直接用 `.form()` 提交，reqwest 的 form 编码会把 `+` 当作空格，导致 `1101 invalid code`。因此**手工拼接 form body，逐字段 `enc()`**：

```
POST https://oauth-login.cloud.huawei.com/oauth2/v3/token
Content-Type: application/x-www-form-urlencoded

grant_type=authorization_code
&code={enc(code)}                  ← 必须编码！authorization_code 含 + / =
&client_id=118065481
&client_secret={enc(secret)}
&redirect_uri={enc(redirect_uri)}
&code_verifier={enc(verifier)}
```

- 每个字段值单独过 `enc()`（`=` `+` `/` 等全部 percent-encode）。
- 字段之间用 `&` 连接，键名（`grant_type`、`code` 等）不编码。

**响应解析**：

- 必须含 `access_token`，否则取 `error_description`（缺省 `error`）报错。
- `TokenPair::from_token_response`：
  - `expires_in` 容忍 int/float（部分返回是 `3599.0`），缺省 `3600`。
  - `expires_at = now_ms + expires_in * 1000`。
  - `refresh_token` 缺省为 `""`（空串，不是 null）。

响应字段：`access_token`、`refresh_token`、`expires_in`（秒）、`token_type`、`scope`。

### 1.5 validate_callback（7 步 authorize 流程）

完整 `authorize()` 严格按以下顺序：

1. **state/PKCE 生成** + 存 `current_verifier`（内存，用于后续换 token 校验）。
2. `OauthServer::start` 启动 loopback 回调服务器。
3. `build_authorize_url` + `open_browser`（macOS 用 `open` 命令拉起默认浏览器）。
4. `wait_for_callback`（300s 超时，三条返回路径均先 `stop()`）。
5. 检查 `cancelled` 标志（用户中途取消）。
6. `validate_callback`：
   - **error 处理**：若回调带 `error` 字段——`1101` 且 `error_description` 含 `"scope"` → 提示去 AGC 后台开通 scope；其他 error 直接报出。
   - **code 必须存在**：缺 `code` → 报错。
   - **state 严格相等**：回调 state 与生成 state 用 `==` 严格比较；不等 → `auth_state_mismatch`（防 CSRF，直接失败，不尝试恢复）。
7. `exchange_code_for_token` → `token_store.save` → `refresher.set_current`（建立内存 current）。

---

## 二、Token 自动刷新（TokenRefresher + Singleflight 并发去重）

### 2.1 触发时机

| 类型 | 触发条件 |
|---|---|
| 主动 | `will_expire_within(60)`：`now_ms + 60*1000 >= expires_at` |
| 被动 | HTTP 401 触发 `execute_with_retry` 重放（强制刷新后重发原请求） |
| 启动 | `restore()` 时若 `will_expire_within(60)` 也触发刷新 |

### 2.2 Singleflight 并发去重（重点）

多个并发请求同时遇到 401 或同时临近过期时，避免发起 N 次刷新。结构：

```rust
struct RefreshSingleflight {
    active: Mutex<Option<Arc<RefreshFlight>>>,
}
struct RefreshFlight {
    result:   Mutex<Option<AppResult>,   // leader 写入结果
    completed: Notify,                    // follower 等待
}
```

`run(operation)` 协调逻辑：

- **首个调用者（leader）**：`active` 为空 → 创建 `RefreshFlight`，存入 `active`，`is_leader = true` → 执行 `operation().await` → `complete`（写 result + `notify_waiters` + `clear_if_active`）。
- **其余调用者（follower）**：`active` 已存在 → clone `Arc<RefreshFlight>`，`is_leader = false` → 等待 leader 完成。

follower 等待循环（防 notify 丢失）：

```
loop {
    if let Some(result) = flight.result.lock().take_clone() { return result }
    flight.completed.notified().await
}
```

> **关键**：每次先检查 `result` 再 `await`。因为 `Notify::notify_waiters` 不排队——若 follower 还没 `await` 时 leader 已 notify，notify 会丢失。先检查 result 可确保不漏。

**`RefreshLeaderGuard::Drop`（防 follower 永久阻塞）**：leader 在 complete 之前被取消（任务 abort）时，Drop 守卫发布 `token_refresh_failed("刷新任务被取消")` 写入 result 并唤醒所有 follower，避免 follower 永久挂起。

**`clear_if_active`（防误清新 leader）**：仅当 `active` 仍 `Arc::ptr_eq` 当前 flight 时才清空。若期间已被新 flight 替换（极端竞态），不清。

### 2.3 perform_refresh

| 项 | 实现 |
|---|---|
| body 编码 | 用 `.form()`（refresh_token 无特殊字符，无需手工编码） |
| 请求 | `POST .../oauth2/v3/token`，`grant_type=refresh_token` + `refresh_token` + `client_id` + `client_secret` |
| refresh_token 回填 | 华为刷新响应**可能不含新 refresh_token** → 沿用 `current.refresh_token` |
| `expires_in` | 容忍 int/float，缺省 `3600` |
| `token_type` | 缺省 `"Bearer"` |
| `scope` | 缺省取 `current.scope` |
| 持久化顺序 | **先 `token_store.save(&new)` 再更新内存 current**（原子持久化优先，崩溃也不丢） |

**传输错误分类**（映射到 `error_kind`）：

- `is_connect` → `Connect`
- `is_timeout` → `Timeout`
- `is_body` → `ResponseBody`

**`restore_refresh_failure_action`**：刷新失败后的处置——**仅 `DriveApiErrorCode::Network` 保留 token 并返回错误**（网络问题，稍后可重试）；其余刷新失败（Auth/Server 等）**一律 `logout()`**（token 已失效，强制重新登录）。

---

## 三、Token 加密存储（token.bin，精确字节布局）

文件位置：`<Application Support>/<bundle id>/token.bin`（dev 使用 `...PetalLink-dev` 独立目录隔离）。

### 3.1 文件布局

```
[魔数 4B = b"PTL1"][nonce 12B 随机][ChaCha20Poly1305 密文 + 16B Poly1305 tag]
```

- **魔数**：4 字节字面量 `b"PTL1"`，用于格式识别。
- **nonce**：12 字节，**每次 save 重新随机**（绝不复用，ChaCha20Poly1305 的 nonce 复用会破坏机密性）。
- **密文**：明文经 ChaCha20Poly1305 AEAD 加密，末尾附 16 字节 Poly1305 tag（完整性）。

### 3.2 明文序列化布局（length-prefixed 小端）

```
[u64 access_len ][access_bytes  ]
[u64 refresh_len][refresh_bytes ]
[i64 expires_at ]                        // 8 字节，毫秒
[u32 token_type_len][token_type_bytes]   // ⚠️ token_type 用 u32（不是 u64！）
[u8 scope_present]                       // 1 = 有 scope, 0 = 无
  若 scope_present == 1:
    [u64 scope_len][scope_bytes]
```

> **重构陷阱（重点）**：`access` / `refresh` / `scope` 的长度前缀用 **u64**（8 字节），但 `token_type` 的长度前缀用 **u32**（4 字节）。Kotlin 用 `DataOutputStream`/`ByteBuffer` 重写时务必混用 `writeLong` 与 `writeInt`，不能统一用一种。`expires_at` 是 **i64 毫秒**（带符号，与 `Instant.toEpochMilli()` 对齐）。

### 3.3 密钥派生

| 步骤 | 实现 |
|---|---|
| `machine_uuid()` | 执行 `ioreg -d2 -c IOPlatformExpertDevice`，解析输出中含 `IOPlatformUUID` 的行，提取 UUID 字符串 |
| `derive_key(uuid)` | `Sha256(uuid.as_bytes())` → 32 字节密钥。**不加 salt，不用慢哈希（PBKDF2/Argon2）** |
| 加密算法 | ChaCha20Poly1305（AEAD），nonce 每次 save 重新随机 12 字节 |

> 设计取舍：放弃 macOS Keychain（签名变化、dev↔release 切换导致 token 不可靠恢复），改用自定义加密文件。UUID 非秘密（本机任意进程可读），所以这是「绑定机器」而非「对抗本机攻击」的方案。

### 3.4 原子写 + 权限

- **save**：写临时文件 `token.bin.tmp` → `set_permissions(0o600)` → `rename` 覆盖（原子替换，崩溃也不会留下半截文件）。
- **clear**：文件不存在视为成功（**幂等**，可重复调用）。
- **失败行为**：load 时读取/解密失败一律返回 `Ok(None)`（视为未登录），只有**路径错误**（如 Application Support 不存在）才向上传播 IO 错误。
- **绝不日志输出 token**（包括截断形式也不打）。

### 3.5 安全边界

| 威胁 | 防护 |
|---|---|
| 跨机器复制 token.bin | ✅ UUID 不同 → AEAD 解密失败 → 视为未登录 |
| 篡改密文 | ✅ Poly1305 完整性校验，改一 bit 即失败 |
| 本机攻击 | ⚠️ 不防——`IOPlatformUUID` 非秘密，本机任意进程可读 |
| 重装系统 | UUID 变 → 视为未登录（需重新登录） |

### 3.6 Kotlin 重构方案选择

| 平台 | 方案 | 取舍 |
|---|---|---|
| macOS（保留方案） | 沿用 ChaCha20-Poly1305 + IOPlatformUUID（JNA/JNI 调 `ioreg` 或直接读 sysfs 等价物） | 与 Rust 行为完全一致，dev/release 隔离简单 |
| macOS（简化） | macOS Keychain（`security` CLI 或 Keychain API） | 需处理 dev/release 签名隔离，否则切换身份时 token 不可恢复 |
| Android | Android Keystore + `EncryptedFile`（Jetpack Security） | 平台原生，依赖硬件密钥 |

---

## 四、Client 凭据注入

**绝对原则**：不能硬编码、不能提交 git。

### 4.1 三级解析优先级（高 → 低）

1. **构建期环境变量**：`build.rs` 从 `.env` 读取 `HWCLOUD_CLIENT_ID` / `HWCLOUD_CLIENT_SECRET`，通过 `rustc-env` 注入；源码用 `option_env!()` 在编译期可见。
2. **运行时 `.env`**：`dotenvy` 运行期加载（覆盖构建期）。
3. **占位符** `PLACEHOLDER_SECRET`：前两级都缺失时的哨兵值，登录必被 AGC 拒绝（用作显式失败而非 panic）。

- `build.rs` 缺失任一凭据 → **`panic` 阻断编译**（强制开发者补全 `.env`）。
- `resolved_client_id()` / `resolved_client_secret()`：按上述优先级合并返回最终值。

### 4.2 Kotlin 实现

- 构建期：根目录 `build-plugin` 只生成不含账号凭据的 `BuildInfo`。
- 运行期：从进程环境或仓库根目录 `.env` 读取 Client 凭据，`.env` 不入库。
- 不设有效凭据的硬编码默认值；缺失时禁用登录流程并显式提示配置错误。

### 4.3 .env 模板

```
HWCLOUD_CLIENT_ID=
HWCLOUD_CLIENT_SECRET=
```

获取方式：AGC 控制台 → 创建 Web 应用 → OAuth 2.0 客户端 → 填回调 `http://127.0.0.1:9999/oauth/callback` → 开通「云空间 / drive」scope。

---

## 五、用户信息聚合（三端点并行）

登录成功后聚合用户展示信息，三个端点 `tokio::join!` 并行，**任一失败不阻断**。

| 端点 | 方法 | 参数 | 备注 |
|---|---|---|---|
| `oauth2/v3/userinfo` | GET | `Authorization: Bearer {access_token}` | OIDC `sub`，**常 404，静默跳过** |
| `rest.php?nsp_svc=GOpen.User.getInfo` | POST form | `access_token`, `getNickName=1` | 昵称 / 头像 / openID，需 `profile` scope |
| `rest.php?nsp_svc=GOpen.User.getPhone` | POST form | `access_token` | 手机号，响应**可能纯文本或 JSON** |

完整 URL：
- `https://oauth-login.cloud.huawei.com/oauth2/v3/userinfo`
- `https://account.cloud.huawei.com/rest.php?nsp_svc=GOpen.User.getInfo`
- `https://account.cloud.huawei.com/rest.php?nsp_svc=GOpen.User.getPhone`

**合并优先级**：OIDC userinfo → info → phone（后者 `extend` 覆盖前者，**phone 最优先**，用于脱敏手机号展示）。

**UserInfo 多别名 pick**（容忍不同端点字段名差异）：
- 用户标识：`sub` / `user_id` / `userId`
- openID：`openID` / `openId` / `open_id`
-（其余字段同理，按别名表逐个尝试）

**`primary_label` 优先级**（选第一个非空作为主显示名）：

```
displayName > mobile > name > nickname > open_id > sub
```

**`resolve_anonymous_as_mobile`**：当用户设为匿名（`displayNameFlag == 1`）且 `mobile` 非空 → `display_name = None`（不在 UI 显示名字，仅展示脱敏手机号）。

---

## 六、占位符模型（macOS 专属，重点修正）

详见 `04-数据模型` §9。为支持「按需下载」（Files-On-Demand-lite），云端文件在本地以**占位文件**形式存在：真实文件名、0 字节空文件、附 xattr 标记，实际内容按需下载。

> 本章已按 **inode 方案**（`schemaVersion=6`）修订。文件身份识别从「fileId xattr」改为「inode + DB 映射」，xattr 键从 5 个精简到 2 个。完整方案见 `11-基于inode的文件身份识别方案.md`。

### 6.1 xattr 键（2 个）

| 键 | 取值 | 作用 |
|---|---|---|
| `com.hwcloud.state` | `placeholder` / `downloaded` | **唯一权威判据（source of truth）** |
| `com.apple.FinderInfo` | `buf[9] = 0x02` | Finder 灰标（label index 7），仅视觉 |

> ⚠️ **inode 方案变更（schemaVersion=6）**：原 `com.hwcloud.fileId` / `com.hwcloud.size` / `com.hwcloud.freeUpRelativePath` 三个 xattr 键已删除。
> - 文件身份（inode → fileId 映射）改由 DB `local_inode_map` 表承担，由 `identity` 模块的 `upsertMapping` 维护。
> - 释放空间恢复路径改由 DB `free_up_staging` 表记录。
>
> 详见 `11-基于inode的文件身份识别方案.md`。

> 注：xattr 命名带 `hwcloud` 前缀，为项目历史命名遗留，仅作键名。

### 6.2 绝对原则

> **`com.hwcloud.state` 是占位状态的唯一权威判据。** 文件大小（0 字节）、文件名、Finder 灰标等都只是伴随特征，**单独不足以判定**。
>
> - 0 字节用户空文件（如 `.gitkeep`、空配置）**不是占位符**——`is_placeholder_file` 只认 xattr `state`，不看文件大小。
> - Finder 灰标（`com.apple.FinderInfo`）仅作视觉提示，**绝不用于判定占位状态**。

### 6.3 状态流转 API

| 方法 | 语义 |
|---|---|
| `create_placeholder_if_needed` | 带存在性检查：已存在且 owner = fileId → skip（幂等）。创建时：**写 `state` xattr + FinderInfo 灰标 + DB 写 inode 映射（`identity.upsertMapping`）** |
| `create_placeholder_strict` | 严格版，破坏性流程专用：**不做检查直接 `create_new`，已存在则报错**。同样写 state + 灰标 + upsertMapping |
| `mark_downloaded` | `state` → `downloaded` + 清除 Finder 灰标。**下载覆盖产生新 inode → 调用 `upsertMapping` 更新映射**（确定性记账，见 `11` §五） |
| `backup_modified_placeholder_if_needed` | `state = placeholder` 且 `size > 0` → 改名为 `.local-<时间戳>`，清备份的 `state` xattr（fileId xattr 已不存在，副本天然产生新 inode） |
| `delete_local` | 0 字节文件**必须是 placeholder 才删**（保护用户空文件） |

### 6.4 改后下载保护

占位被用户写入内容（`state = placeholder` 但 `size > 0`）时，下载云端版本前：

1. 将本地这份修改改名备份：`<basename>.local-<YYYYMMDD-HHMMSS>.<ext>`（撞名追加 `.seq`）。
2. **清备份文件的 `state` xattr**（否则下轮 planner 会误判「云端删除」而删掉备份）。fileId xattr 在 inode 方案下已不存在，副本改名天然产生新 inode，无需额外清理。
3. 随后下载云端版本到原路径。

### 6.5 状态流转总览

| 事件 | 操作 |
|---|---|
| 创建占位 | 云端有本地无 → 创建 0 字节文件 → 写 `state` xattr + FinderInfo 灰标 + DB 写 inode 映射（`identity.upsertMapping`） |
| 下载完成 | xattr `placeholder` → `downloaded` + 清灰标；下载覆盖产生新 inode → `upsertMapping` 更新映射 |
| 释放空间 | 删本地真实内容 → 重建占位 → 状态 `placeholder`；恢复路径来源从 xattr 改为 `free_up_staging` 表 |
| 改后下载保护 | 占位被用户写入（state=placeholder 但 size>0）→ 改名备份 + 清备份 `state` xattr |

### 6.6 recover_interrupted_free_up

启动时收敛上次中断的「释放空间」事务：

1. **读 `free_up_staging` 表**（替代扫描暂存文件的 `XATTR_FREE_UP_RELATIVE_PATH` xattr），取出所有暂存记录（含 `staging_name` / `relative_path`）。
2. 对每条暂存记录定位暂存文件（`.hwcloud_freeup-` 前缀）。
3. 判定 DB 是否已提交该项：
   - **已提交** → 删除暂存（事务已完成，暂存是残留）。
   - **未提交** → 恢复暂存回原路径（回滚未完成事务）。

> 好处：暂存文件与恢复记录在同一 DB 事务内，消除「文件已暂存但 xattr 未写」的窗口（见 `11` §4.8）。

### 6.7 安全守卫与已知限制

- 文件已存在但**无 `state` xattr** → 视为用户文件，**绝不**转换为占位（防误吞用户数据）。
- 0 字节且非占位 → 拒绝删除（保护用户空文件）。
- Finder 灰标仅视觉提示，**绝不用于判定占位状态**。

已知限制：
- `state` xattr 无完整性校验。用户手动篡改 xattr 会改变系统认知（但仅影响「占位 vs 已下载」的本地标记，不影响云端身份——身份由 inode + DB 映射承担）。
- inode 在下载覆盖 / 跨卷移动 / 断电恢复等场景可能漂移；方案对「下载覆盖」做确定性兜底（`upsertMapping`），对「断电/还原漂移」用 size + mtime 二级校验保守确认（见 `11` §6）。
- Finder 复制会让副本携带相同 `state` xattr（占位符复制还是占位符，这是合理行为）；副本产生新 inode，DB 自动当独立文件处理，不再有「同一 fileId 多处出现」的歧义。

---

## 七、退出登录清理（F-AUTH-06）

退出登录必须清理（5 步，缺一不可）：

1. **`token.bin`**（加密 token 文件，`token_store.clear`）。
2. **内存 token**（`TokenRefresher` current 清空）。
3. **DB 记录**（`sync_items` + `transfer_queue` 全表 `delete_all`；inode 方案下另需清空 `local_inode_map` + `free_up_staging`，见下方注）。
4. **缓存文件**（`cloudtree` / `syncstate` / `changes_cursor`，经 `clear_all_cache_files`）。
5. **config 挂载字段重置**：`mount_dir = ""`、`mount_configured = false`，**其余设置保留**（concurrency、pollInterval 等用户偏好不动）。

> **inode 方案注（schemaVersion=6）**：第 3 步建议一并清空 `local_inode_map`（inode→fileId 映射）与 `free_up_staging`（释放空间暂存）两张表。这两张表都是**运行期数据**，清空后可由下次扫描自动重建，不属于同步基线。即使不清空，重新登录后首次扫描也会覆盖陈旧记录，因此该清理为**可选**（运行期数据可重建）。

### 7.1 孤儿状态清理（F-AUTH-07）

启动加载 config 前，检测并清理孤儿状态：

- **守卫**：`mount_configured` 必须为 `true` 才执行清理（避免误清新装）。
- **触发**：token 丢失（`token.bin` 不存在或解密失败）但 config 残留挂载配置 → 调用 `cleanup_orphan_state`。
- **首装**：config 默认 `mount_configured = false` → 跳过清理。

---

## 八、日志安全

### 8.1 三层输出

| 层 | 实现 |
|---|---|
| stdout | 控制台输出 |
| 滚动文件 | 每日轮转 `PetalLink.log`，`cleanup_old_logs` 保留 **30 天** |
| 环形缓冲 | `MAX_BUFFER_SIZE = 1000`，newest-first，供设置页查看 |

### 8.2 过滤与脱敏

- `EnvFilter`：默认 `info` 级别。**debug 模式也用 INFO**（避免对 17K 文件做 BFS 时产生数万条 FINE 日志）。
- **绝不打印完整 token / secret**（含截断形式也不打）。
- 错误消息只含**用户可读中文**描述，不泄露协议细节、URL、内部错误码给终端用户。
