# 03 · 华为 Drive REST API 接口规范

> ⚠️ **这是整个项目最难、最易踩坑的部分。** 华为服务端与标准协议（GDrive/OIDC）有多处偏差，以下每条处理都经过端到端验证。
> Kotlin 重构必须完整复刻这些处理，否则登录失败 / 中文乱码 / 上传错误 / 同步震荡。
>
> 本文基于原项目 `src/drive/` 全部 16 个源文件 + `src/auth/` 8 个源文件的逐行核对。

---

## 域名汇总

| 用途 | 域名 |
|---|---|
| OAuth 认证 | `oauth-login.cloud.huawei.com` |
| 账号信息 | `account.cloud.huawei.com` |
| Drive REST API（CRUD/搜索/缩略图/下载） | `driveapis.cloud.huawei.com.cn/drive/v1` |
| 上传 API（multipart + resume） | `driveapis.cloud.huawei.com.cn/upload/drive/v1`（与 drive 是**兄弟路径**，非父子） |

## 认证方式

所有 Drive/Upload API 请求均需携带：
```
Authorization: Bearer {access_token}
```
**例外**：Token 端点（`/oauth2/v3/token`）**不注入 auth**（URL 含 `oauth2/v3/token` 时跳过），否则导致循环刷新。

## 通用请求头

| Header | 值 |
|---|---|
| `Authorization` | `Bearer {access_token}` |
| `Content-Type` | `application/json`（除上传用 multipart/related） |
| `Accept` | `application/json` |

---

## 一、HTTP 客户端架构（client.rs）

### 1.1 客户端构造

```kotlin
// Kotlin 等价示意
HttpClient {
    connectTimeout = 15.seconds
    timeout = 60.seconds          // 普通请求
    poolMaxIdlePerHost = 15
}
// Upload 客户端单独构造：timeout = 120.seconds + redirect(Policy::none)（禁用自动重定向）
```

### 1.2 execute_with_retry（统一发送 + 401 重放）

```
1. semantics = request_semantics(method)  // GET/HEAD/OPTIONS=Read，其余=Write
2. 第一次发送：build_authed(method, url).await → apply → send
   - 传输错误用 classify_transport_error(error, semantics, auth_already_replayed=false)
3. 响应 != 401 → ensure_success_response
4. 401 → auth.refresher().refresh() → build_authed_with_token(新token) 重放一次
   - 重放错误用 auth_already_replayed=true
```

- `build_authed`：URL 含 `oauth2/v3/token` 时不注入 auth；否则 `ensure_valid_access_token()`（距过期<60s 主动刷新）+ bearer
- `build_authed_with_token`：直接用给定 token，不再 ensure（重放专用）

### 1.3 错误分类（classify_transport_error）

kind 优先级：`is_connect > is_timeout > is_body > is_decode > is_request > Other`

**关键副作用规则**：
```
request_may_have_reached_server = semantics.is_write() && transport_kind != Connect
```
（写操作 + 非连接失败 = 请求可能已到达服务端 → 走 VerifyRemote 核验）

### 1.4 Retry-After 解析（parse_retry_after）

- delta-seconds（纯数字）→ `DelaySeconds(u64)`
- RFC2822 日期 → `AtUnixMs(timestamp_ms)`
- 用于 429 / 503 退避计算

---

## 二、认证端点

### 1. OAuth 授权页

| 项 | 内容 |
|---|---|
| 端点 | `GET https://oauth-login.cloud.huawei.com/oauth2/v3/authorize` |
| 必选参数 | `response_type=code`, `client_id=118065481`, `redirect_uri=http://127.0.0.1:{port}/oauth/callback`, `state={32字节随机hex}`, `access_type=offline`, `code_challenge={PKCE S256}`, `code_challenge_method=S256`, `scope=openid%20profile%20https://www.huawei.com/auth/drive` |
| ⚠️ scope 编码 | `SCOPES.join(" ")` 后 `.replace(' ', "%20")`，**`/` 不编码**（否则报 1101 invalid scope） |
| 参数顺序 | 固定：response_type → client_id → redirect_uri → state → access_type → code_challenge → code_challenge_method → scope（最后） |

### 2. 授权码换 Token

| 项 | 内容 |
|---|---|
| 端点 | `POST https://oauth-login.cloud.huawei.com/oauth2/v3/token` |
| Content-Type | `application/x-www-form-urlencoded` |
| 必选参数 | `grant_type=authorization_code`, `code={授权码}`, `client_id`, `client_secret`, `redirect_uri`, `code_verifier` |
| ⚠️ 关键细节 | **必须手工拼 form body + 逐字段 `enc()` 精确编码**（不用 `.form()`）。授权码含 `+ / =` 特殊字符，`+` 必须编码为 `%2B`（否则被 form-urlencoded 当空格 → 1101 invalid code） |
| `enc()` | 等价 dart `Uri.encodeComponent`：RFC3986 unreserved `A-Za-z0-9-_.~` 不编码，其余全编码 |
| 响应 | `access_token`, `refresh_token`(可能缺省为""), `expires_in`(秒，容忍 int/float), `token_type`, `scope` |

### 3. Token 刷新

| 项 | 内容 |
|---|---|
| 端点 | 同上 `/oauth2/v3/token` |
| 方法 | POST form-urlencoded（**用 `.form()`**，refresh_token 无特殊字符） |
| 触发 | access_token 距过期 < 60s 主动刷新；HTTP 401 后强制刷新重放 |
| 必选参数 | `grant_type=refresh_token`, `refresh_token`, `client_id`, `client_secret` |
| ⚠️ 关键细节 | 华为刷新响应**可能不含新 refresh_token** → 沿用旧的；`expires_in` 容忍 int/float，缺省 3600 |
| 并发去重 | **Singleflight** 机制：首个调用者为 leader 执行刷新，其余为 follower 等待结果（见 `07-安全` §二） |

### 4-6. 用户信息（三端点并行）

| # | 端点 | 方法 | 参数 | 用途 | 备注 |
|---|---|---|---|---|---|
| 4 | `oauth-login.cloud.huawei.com/oauth2/v3/userinfo` | GET | bearer | sub | **常 404，静默跳过** |
| 5 | `account.cloud.huawei.com/rest.php?nsp_svc=GOpen.User.getInfo` | POST form | `access_token`, `getNickName=1` | 昵称/头像/openID | scope 需 `profile`；`getNickName=1` 返回真实昵称 |
| 6 | `account.cloud.huawei.com/rest.php?nsp_svc=GOpen.User.getPhone` | POST form | `access_token` | 手机号 | **body 可能纯文本或 JSON**：先试 JSON 解析，失败则包装为 `{"mobile": text}` |

- `tokio::join!` 并行，任一失败不阻断
- 合并优先级：oidc → info → phone（extend 覆盖，phone 最优先）

---

## 三、Drive 网盘 API

### 7. 配额信息

| 项 | 内容 |
|---|---|
| 端点 | `GET /drive/v1/about?fields=*` |
| ⚠️ | `fields=*` 为**强制**参数（否则 400）；配额在 `storageQuota` 子对象，且为 **String 类型**需容忍解析（`tolerant_parse_int` 接受 int/num/String） |
| 响应 | `storageQuota.userCapacity`(总, String), `storageQuota.usedSpace`(已用, String), `user.displayName` |

### 8. 列举文件（单页）

| 项 | 内容 |
|---|---|
| 端点 | `GET /drive/v1/files?fields=*&pageSize=100&queryParam='{folderId}' in parentFolder` |
| ⚠️ | **不用 `parentFolder` 参数！** 华为只认 `queryParam='id' in parentFolder` 语法。根目录用 `'root'`。**单引号必须存在** |
| folder_token | None/空/`"root"` → `"root"`，其余 trim 校验 |
| 翻页 | `cursor={nextCursor}`（enc 编码） |
| pageSize | 1-100（`PRODUCTION_PAGE_SIZE=100`），`validate_page_size` 校验 |

### 9. 列举文件（全量翻页 list_all）

- 固定 pageSize=100 循环至 `nextCursor` 为空
- 用 `HashSet` 检测 cursor 重复/循环（重复→错误）
- 达 max_pages(1000) 仍 nextCursor → 错误（**绝不返回部分结果**）
- `parse_file_list_page` 严格解析：`category` 若出现必须 `drive#fileList`；`files` 必须数组（缺失/非数组→整页失败）

### 10. 获取文件元数据

`GET /drive/v1/files/{percent_encode(fileId)}?fields=*`。upload resume 尾部兜底确认也用此。

### 11. 创建文件夹

| 项 | 内容 |
|---|---|
| 端点 | `POST /drive/v1/files?fields=*` |
| 请求体 | `{ "fileName": "名称", "mimeType": "application/vnd.huawei-apps.folder", "parentFolder": ["{parentId}"] }` |
| ⚠️ | ① `mimeType` 必填（否则 21004001）② root 目录**省略** `parentFolder` ③ 中文名必须 ASCII 转义（否则 21004002） |
| **非幂等** | 写前先 `find_unique_folder_in_parent`（list_all 过滤同名+folder）查重；命中唯一→跳过返回。POST 失败后再查重一次：唯一命中→视为已提交 |

### 12. 更新文件（重命名/移动/改描述）

| 项 | 内容 |
|---|---|
| 端点 | 重命名/描述：`PATCH /drive/v1/files/{fileId}?fields=*`；移动：追加 `addParentFolder={enc(newId)}&removeParentFolder={enc(oldId)}`（成对 query 参数，**不在 body 写 parentFolder**） |
| 请求体 | `{ "fileName": "新名", "description": "..." }`（body 用 `ascii_json_encode` 转义中文） |
| ⚠️ | 成功必须 **`200`**（`require_official_write_ok` 仅接受 200，其他 2xx 也拒绝）+ `File`，核验同一 id 及目标 name/唯一 parent；响应丢失按 fileId GET 核验 |
| 移动前 | 先 GET 当前 parent（让重复调用具 fileId 级幂等），同 parent 且无 rename→直接返回 |

### 13. 删除文件（移入回收站）

| 项 | 内容 |
|---|---|
| 端点 | `PATCH /drive/v1/files/{fileId}` |
| 请求体 | `{"recycled": true}` |
| ⚠️ | **`DELETE` 是永久删除，不用。** 软删除成功合同：**HTTP 200 + File.id==请求 id + recycled=true**（明确布尔）；响应丢失时 GET 得 404 或 recycled=true 才结算 |

### 14. 搜索文件

| 项 | 内容 |
|---|---|
| 端点 | `GET /drive/v1/files?fields=*&pageSize=100&queryParam={enc(query)}` |
| DSL | `fileName contains 'keyword'`，可叠加 `and 'parentId' in parentFolder`；整段 query 只 URL encode 一次 |
| ⚠️ | `validate_query_literal`：**拒绝含 `'` 或 `\` 的输入**（华为 DSL 未定义转义规则，fail closed） |

### 15. 缩略图

`GET /drive/v1/thumbnails/{fileId}?form=content`。**用 raw_http + 手动 bearer**（不走 401 重放），返回二进制图片数据。

### 16. 增量变更 — 获取初始 cursor

| 项 | 内容 |
|---|---|
| 端点 | `GET /drive/v1/changes/getStartCursor?fields=*` |
| ⚠️ | 华为 `/changes` 强制要求 cursor，无 cursor 直接 400。初始 cursor **必须**先通过本端点获取 |
| 响应 | 校验 `category==drive#startCursor`；`startCursor` 非空（纯数字字符串，非 GDrive 长 token） |

### 17. 增量变更 — 拉取变更列表

| 项 | 内容 |
|---|---|
| 端点 | `GET /drive/v1/changes?fields=*&pageSize=100&includeDeleted=true&cursor={enc(cursor)}` |
| 必选 | `cursor`（必填且不能为空） |
| 分页 | **`nextCursor`（翻页）** 与 **`newStartCursor`（末页 checkpoint）** 语义不同，**禁止合并** |
| 追平判定 | 只能以「末页 + 有效 newStartCursor」结束；**空的中间页仍继续**（nextCursor 非空就翻） |
| ⚠️ 错误 | cursor 无效 → 400；cursor 过期 → 410（`21084100` CURSOR_EXPIRED）。失败保留旧 checkpoint，回退全量 |

**Change 严格解析**（`Change::from_json`，任一字段无法安全解释→整页失败）：
- `category` 若出现必须 `drive#change`；`type` 若出现必须 `File`；`time` 若出现必须 RFC3339
- `fileId` 必须非空字符串；`file.id` 必须等于 `fileId`（不一致→错误）
- **三种删除信号**：`deleted==true` **或** `changeType=="trashDone"` **或** `file.recycled==true` → Removed
- Modified 必须有可完整解析的 file + **且只能有一个 parentFolder**（数量 !=1 →错误）

```json
// 响应示例
{
  "category": "drive#changeList",
  "changes": [{
    "changeType": "trashDone",
    "deleted": false,
    "file": { "recycled": true },
    "fileId": "ADz3nes6G34...",
    "time": "2026-07-06T05:51:13.053Z"
  }],
  "newStartCursor": "311298"
}
```

**list_all_changes 完整一轮 catch-up**：
- 用 HashSet 检测 cursor 重复/循环
- 达 max_pages(10000) 仍有 nextCursor → 错误
- 终页必须有非空 newStartCursor（缺失→错误）；newStartCursor 未推进或循环 → 错误

---

## 四、文件上传

### 常量阈值

| 常量 | 值 | 说明 |
|---|---|---|
| `SMALL_LARGE_THRESHOLD` | 20MB | 小/大文件分界 |
| `SAFE_EXISTING_UPDATE_MAX_BYTES` | 20MB | Update 安全上限（超过拒绝） |
| `MIN_CHUNK_SIZE` | 256KB | 分片最小 |
| `DEFAULT_CHUNK_SIZE` | 2MB | 分片默认（sliceSize 缺省时） |
| `MAX_CHUNK_SIZE` | 64MB | 分片最大 |
| `CHUNK_RETRIES` | 3 | 分片连接失败重试 |
| `FINAL_STATUS_MAX_POLLS` | 5 | 最终状态轮询上限 |
| `FINAL_STATUS_POLL_INTERVAL_SECS` | 3 | 轮询间隔 |

### 18. 小文件上传（≤20MB）

| 项 | 内容 |
|---|---|
| 端点 | `POST /upload/drive/v1/files?uploadType=multipart` |
| Content-Type | **`multipart/related; boundary=hwcloud_{timestamp_micros}`**（不是 form-data） |
| ⚠️ | 第 1 部分 `application/json`（metadata，**容忍 UTF-8，不转义**），第 2 部分 `application/octet-stream`（二进制） |
| metadata | `build_metadata_json`：普通 JSON `{fileName, parentFolder:[pid]}` |
| 配额 | 上传前 `ensure_capacity` GET about 校验剩余空间 |

**multipart/related body 结构**：
```
--{boundary}\r\n
Content-Type: application/json; charset=UTF-8\r\n\r\n
{metadata}
\r\n
--{boundary}\r\n
Content-Type: application/octet-stream\r\n\r\n
{file_bytes}
\r\n--{boundary}--\r\n
```

### 19. 大文件分片上传 — 初始化会话

| 项 | 内容 |
|---|---|
| 端点 | `POST /upload/drive/v1/files?uploadType=resume` |
| 请求头 | `X-Upload-Content-Length: {totalSize}` + `Content-Type: application/json` + bearer |
| 请求体 | `{ "fileName": "name", "parentFolder": ["{parentId}"] }` |
| 响应 body | `{"sliceSize": 10485760}`（仅推荐分片大小，**不含 serverId/uploadId**） |
| ★ Location 头 | 会话 URL：`https://driveapis.cloud.huawei.com.cn/upload/drive/v1/{token}/files?uploadType=resume&uploadId={id}`。**后续所有分片 PUT 直接用此 URL** |
| ⚠️ | **从 Location 响应头提取会话 URL**。无 session_url 且无 serverId → 错误（不回退新建第二个会话） |
| server_id | 从 body `serverId/id/fileId` 兼容取（仅作记录） |
| init 401 | 刷新 token 后**重试一次 init**；init 其他失败 → 保留结构化错误，**绝不新建第二个会话** |

### 20. 大文件分片上传 — 上传分片

| 项 | 内容 |
|---|---|
| 端点 | **直接用 init 响应 Location 头的 URL**（或 `{upload_base}/files/{server_id}?uploadId={upload_id}`） |
| 方法 | PUT |
| 请求头 | `Content-Range: bytes {offset}-{end}/{totalSize}`, `Content-Length: {chunkLen}` |
| ★ 中间响应 | **HTTP 308 Resume Incomplete**，body `{"sliceSize":..., "rangeList":["0-10485759"]}`。**308 是正常响应，不重试** |
| 最终响应 | HTTP 200/201，body 含完整文件元数据 |
| 401 | 刷新后对同一 session 最多重放一次（URL/body/Content-Range 完全不变） |
| ⚠️ 恢复策略 | 308 解析连续 rangeList；连接/超时/5xx/丢响应先查同一 session 状态，**只按服务端确认 offset 前进，禁止用 offset+chunkLen 推算** |

**`parse_confirmed_offset`（最关键算法）**：
- 取 `rangeList` 数组（缺失→错误）；空数组→0
- 遍历每个 `"start-end"` 字符串，要求**从 0 开始、连续、无重叠、不越界**（start==expected_start, end>=start, end<total_size）
- 任一不满足→remote_ambiguity
- 返回 `end+1`

**`should_retry_chunk_locally`**：仅 `status_code==None && Connect && 未到达服务端` 才本地重试（指数退避 attempt 秒）。

### 21. 大文件分片上传 — 查询最终状态

| 项 | 内容 |
|---|---|
| 端点 | init 响应 Location URL |
| 方法 | PUT，`Content-Range: bytes */{totalSize}`, `Content-Length: 0`（空 body） |
| 用途 | 所有分片发完但未拿到元数据时（末片也返回 308） |
| 轮询 | 最多 `FINAL_STATUS_MAX_POLLS=5` 次，间隔 `process_time_ms`（clamp 250..3000ms，缺省 3000） |

### 22. 上传覆盖已有文件（PATCH）

| 项 | 内容 |
|---|---|
| 端点 | `PATCH /upload/drive/v1/files/{fileId}?uploadType=multipart` |
| ⚠️ | **禁止 Update→Create 降级。** `reject_unsafe_large_update`：>20MB 既有文件替换明确拒绝（`restart_required`），保留远端原文件 |
| PATCH 响应不确定 | 按既有 fileId 核验（`verify_remote`） |

### 路由规则（routing.rs）
- `upload(path, parent_id)`：size ≤ 20MB → `upload_small`；> 20MB → `upload_resume`
- Upload 客户端：**timeout 120s + redirect(Policy::none)**（禁用自动重定向，华为 resume 的 308/Location 不应被自动跟随）

---

## 五、文件下载（download_api.rs）

### 路径约定
- `tmp_path(dest) = dest + ".tmp"`（watcher/scanner 忽略）
- `resume_metadata_path(dest) = dest + ".download-meta.tmp"`（断点元数据，也以 .tmp 结尾）
- `resume_metadata_staging_path = dest + ".download-meta-write.tmp"`

### download_with_expectation 流程

1. 创建父目录
2. `fetch_remote_metadata(file_id)`：GET `/files/{enc_id}?fields=*`，**读响应头 ETag**；body 校验 id==file_id；取 size（容忍 String）、editedTime→ms、sha256、etag（header 优先）
3. expectation 校验（edited_time_ms/size/content_hash 全匹配）
4. **validated_resume_offset**：tmp 不存在→0；读 stored metadata，stored != current 或无 stable_identity → discard→0；tmp length > size→discard→0；否则返回 tmp length（**只认 .tmp 实际文件长度，不推算**）
5. 写 resume_metadata（staging→sync→rename 原子写）
6. tmp 已存在且 offset==size → 直接 verify_and_install
7. **Range 下载循环**（`restarted_from_zero` 标志，**只允许一次安全回退**）：
   - `send_content_request(file_id, offset, etag)`：401 刷新重放一次
   - **416 + offset>0 + 未重启过 → discard + 重写 metadata + offset=0 + continue**
   - `validated_response_offset`：**200→0**（服务端忽略 Range，截断从 0 写）；**206→解析 Content-Range** `bytes start-end/total`，校验 start==requested && total==expected && end>=start && end<total
   - write_offset==0→create（截断），>0→append；流式写，每片回调 progress
   - 写完 flush + sync_all；校验 actual_size==size
8. **verify_and_install**：
   - size 复核 → **sha256 流式校验**（1MB buffer）
   - **再 fetch_remote_metadata 一次**（防无 ETag 时把两个云端版本混为一次成功）
   - verify_local_destination（占位符/快照核验，防覆盖用户内容）
   - **`tokio::fs::rename(tmp, dest)`**（POSIX 同文件系统原子替换）
   - remove_resume_metadata

### cleanup_if_permanent
- 仅永久失败清除断点；**暂态失败（无 status_code / 401/408/409/425/429/5xx）保留 .tmp 现场供续传**

---

## 六、关键踩坑清单（18 条，全部已验证）

| # | 怪癖 | 影响 | 处理方式 |
|---|---|---|---|
| 1 | `category` 恒为 `drive#file`，类型在 `mimeType` | List/Get | 用 `mimeType` 判断文件夹（兼容 4 种写法：`vnd.huawei-apps.folder`/`vnd.huawei-app.folder`/`vnd.google-apps.folder`/`x-folder`） |
| 2 | scope `/` 不能 URL 编码 | Authorize | scope 单独拼接，空格→`%20`，`/` 保留 |
| 3 | authorization_code 含 `+`，form-urlencoded 当空格 | Token | **手工拼 form body + 精确编码**（`+`→`%2B`） |
| 4 | 中文名 → application/json 报 400（21004002） | Create/Update | ASCII 转义 `>0x7F → \uXXXX`（含代理对） |
| 5 | 不支持 `parentFolder` 参数，只能用 queryParam | List/Search | `queryParam='id' in parentFolder`（单引号必须） |
| 6 | 配额字段为 String 类型 | About | `tolerant_parse_int` 接受 int/num/String |
| 7 | 仅接受 `multipart/related`（拒绝 form-data） | Upload | GDrive 风格多段 body |
| 8 | 软删除成功合同是 200 + File | Delete + Get | 核验同 fileId 且 recycled=true；不确定 GET 404/recycled 收敛 |
| 9 | Root 不是字面 `"root"`，是真实 folder ID | BFS | `detect_root_folder_id`：统计 parent_folder 高频值，**最高频并列则 fail closed** |
| 10 | `serverId` ≠ `fileId` | Upload Resume | 尾部兜底用 `createdFileId` 查 `/files/{fid}` |
| 11 | 刷新 token 可能不含新 `refresh_token` | Refresh | 沿用旧 refresh_token |
| 12 | Resume init 仅返回 `{"sliceSize":...}` | Upload Resume | **从 Location 响应头提取会话 URL** |
| 13 | `/changes` 强制要求 cursor | Changes | 失效保留旧 checkpoint，全量重建 |
| 14 | 官方 `deleted=true`，本地另抓到 trashDone/recycled | Changes | 三种删除信号兼容；无 File tombstone 不伪造 |
| 15 | `nextCursor` 与 `newStartCursor` 语义不同 | Changes | 前者逐页，后者仅末页提交；空中间页继续 |
| 16 | 分片 PUT 返回 **HTTP 308**，body 含 rangeList | Upload Resume | 严格验证连续范围；不确定写先查同一 session |
| 17 | 全部分片后可能仍无文件元数据 | Upload Resume | 向同一 URL 发 `PUT bytes */{total}`，完整 File 核验后才完成 |
| 18 | OIDC userinfo 常 404 | UserInfo | 静默跳过 |

---

## 七、中文文件名 ASCII 转义（ascii_json.rs，关键算法）

华为 Drive API 服务端 JSON 解析器**不接受 UTF-8 多字节字符**直接出现在 JSON 值中（即使 Content-Type 声明 charset=utf-8），需将所有 `> 0x7F` 码点转义为 `\uXXXX`。

**算法（`escape_non_ascii`）**：
```
对每个 char c (code = c as u32):
  if code > 0x7F:
    if code <= 0xFFFF:                    // BMP
      push "\u{:04x}"                     // 小写 hex，4 位
    else:                                 // 辅助平面 → UTF-16 代理对
      v = code - 0x10000
      high = 0xD800 + (v >> 10)
      low  = 0xDC00 + (v & 0x3FF)
      push "\u{high:04x}\u{low:04x}"      // 两个 \uXXXX
  else:
    push c
```

**关键**：按 char（Unicode scalar）遍历；emoji 等辅助平面字符拆成**两个** `\uXXXX`（代理对）。

> `ascii_json_encode<T: Serialize>` 先 `serde_json::to_string` 再 `escape_non_ascii`。
> **注意**：multipart 上传的 metadata 部分容忍 UTF-8，**不需要转义**；只有 application/json 的 Create/Update 需要。

---

## 八、防振荡守卫

华为 PATCH `recycled:true` 后 LIST 仍可能返回旧数据（最终一致性）。**防振荡守卫 `recentlyDeletedPaths` 位于 `sync/engine`**（非 drive 模块）：

- `recently_deleted_paths: HashMap<String /*relative_path*/, i64 /*ms*/>`
- 写入时机：action 成功且类型为 DeleteFromCloud/DeleteFromLocal/BackupBeforeCloudDelete → `insert(rel, now_ms)`
- TTL 清理：每次 settle 结束 `retain(|_, ts| ts > now_ms - 300_000)`（**5 分钟**过期）
- `filter_anti_oscillation`：丢弃近期删除路径上的回摆动作，**但保留 DeleteFromCloud**（允许继续确认云端删除）

---

## 九、严格 schema 校验（response.rs）

华为返回的 schema 有歧义，以下校验确保歧义永不变成可信空页：

| 校验函数 | 规则 |
|---|---|
| `parse_file_list_page` | 顶层必须对象；`category` 若出现必须 `drive#fileList`；`files` 必须数组（缺失/非数组→整页失败）；`nextCursor` 缺失/null/空串→终页 |
| `parse_drive_file_strict` | id/fileName/mimeType 必须**非空字符串**；size 非负 int 或 null；createdTime/editedTime 必须 RFC3339；parentFolder 非空字符串数组或 null |
| `require_official_write_ok` | **华为 Files 写操作成功状态必须是 200**（其他 2xx 也拒绝） |
| `verify_written_file_id` | 响应 id 必须等于请求 id |
| `single_parent` | **只支持一个非空 parent** |

---

## 十、API 调用链路

### 登录流程
```
点击登录 → authorize(port)
  → generate_state + generate_pkce
  → OauthServer::start(port)  // 绑定 127.0.0.1
  → build_authorize_url → open_browser
  → wait_for_callback(5min 超时)
  → validate_callback(state 严格相等)
  → exchange_code_for_token(手工拼 form body)
  → token_store.save(加密)
  → refresher.set_current
```

### 同步 cycle
```
FSEvents/手动/启动恢复
  → run_sync_cycle_inner
  → scanLocal
  → planner.plan (3-way diff)
  → detect_renames
  → filter_anti_oscillation
  → executor.execute_all
       ├── do_upload    → POST /upload/drive/v1/files (multipart or resume)
       ├── do_download  → GET /drive/v1/files/{id}?form=content
       ├── do_create_placeholder → mountManager
       ├── do_create_folder      → POST /drive/v1/files
       ├── do_delete_from_cloud  → PATCH /drive/v1/files/{id} {"recycled":true}
       ├── do_delete_from_local  → mountManager.deleteLocal
       └── do_conflict   → ConflictResolver
```

### BFS 云端树构建
```
refresh_cloud_tree
  → detect_root_folder_id(高频 parent，并列 fail closed)
  → list_all(root) × 8 并发 → GET /drive/v1/files (loop pages)
  → persist → cloudtree_{escaped}.json (tmp→fsync→rename→fsync父目录)
  → load_persisted → 非首次启动秒级加载 (~200ms)
```
