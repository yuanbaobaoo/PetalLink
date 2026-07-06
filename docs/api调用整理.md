# PetalLink Tauri 重构 —— 华为 API 调用汇总

> 整理日期：2026-06-24
> 重构版本：Tauri 2.x (Rust 后端)

---

## 一、认证与授权（Auth）

### 1. OAuth 2.0 授权页

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://oauth-login.cloud.huawei.com/oauth2/v3/authorize` |
| **方法** | `GET`（通过系统浏览器打开） |
| **调用场景** | 用户点击「使用华为账号登录」按钮时，打开浏览器跳转华为登录页 |
| **必选参数** | `response_type=code`, `client_id=118065481`, `redirect_uri=http://127.0.0.1:{port}/oauth/callback`, `state={32字节随机hex}`, `access_type=offline`, `code_challenge={PKCE S256 challenge}`, `code_challenge_method=S256`, `scope=openid%20profile%20https://www.huawei.com/auth/drive` |
| **scope 编码** | 空格替换为 `%20`，`/` 不编码（否则华为报 1101 invalid scope） |
| **代码位置** | `src/backend/src/auth/service.rs:build_authorize_url()` |
| **官方文档** | https://developer.huawei.com/consumer/cn/doc/HMSCore-Guides/web-get-access-token-0000001050048946 |

### 2. 授权码换 Token

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://oauth-login.cloud.huawei.com/oauth2/v3/token` |
| **方法** | `POST` |
| **Content-Type** | `application/x-www-form-urlencoded` |
| **调用场景** | OAuth 回调收到授权码后，换取 access_token + refresh_token |
| **必选参数** | `grant_type=authorization_code`, `code={授权码}`, `client_id=118065481`, `client_secret={运行时注入}`, `redirect_uri=http://127.0.0.1:{port}/oauth/callback`, `code_verifier={PKCE verifier}` |
| **关键细节** | 授权码含 `+ / =` 特殊字符，必须手工拼接 form body 并用 `Uri.encodeQueryComponent` 等价方式编码（否则 `+` 被当空格 → 1101 invalid code） |
| **响应字段** | `access_token`, `refresh_token`, `expires_in`（秒）, `token_type`, `scope` |
| **代码位置** | `src/backend/src/auth/service.rs:exchange_code_for_token()` |
| **官方文档** | https://developer.huawei.com/consumer/cn/doc/HMSCore-Guides/web-get-access-token-0000001050048946 |

### 3. Token 刷新

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://oauth-login.cloud.huawei.com/oauth2/v3/token` |
| **方法** | `POST` |
| **Content-Type** | `application/x-www-form-urlencoded` |
| **调用场景** | access_token 距过期 < 60s 时自动刷新；收到 HTTP 401 后强制刷新重放 |
| **必选参数** | `grant_type=refresh_token`, `refresh_token={旧token}`, `client_id=118065481`, `client_secret={运行时注入}` |
| **关键细节** | 华为刷新响应**可能不含新 refresh_token** → 沿用旧的；并发刷新去重（同一时刻只有一个刷新在执行） |
| **响应字段** | `access_token`, `expires_in`, `token_type`（`refresh_token` 可选） |
| **代码位置** | `src/backend/src/auth/token_refresher.rs:refresh()` |
| **官方文档** | https://developer.huawei.com/consumer/cn/doc/HMSCore-Guides/web-get-access-token-0000001050048946 |

### 4. OIDC 用户信息

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://oauth-login.cloud.huawei.com/oauth2/v3/userinfo` |
| **方法** | `GET` |
| **调用场景** | 拉取用户唯一标识 sub（登录后获取，常 404） |
| **Header** | `Authorization: Bearer {access_token}` |
| **关键细节** | 华为该端点不完全兼容标准 OIDC，404 是常态 → 静默跳过，不报错 |
| **代码位置** | `src/backend/src/auth/user_info_api.rs:get_oidc_user_info()` |
| **官方文档** | https://developer.huawei.com/consumer/cn/doc/HMSCore-Guides/web-get-access-token-0000001050048946 |

### 5. 显示名称

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://account.cloud.huawei.com/rest.php?nsp_svc=GOpen.User.getInfo` |
| **方法** | `POST` |
| **Content-Type** | `application/x-www-form-urlencoded` |
| **调用场景** | 拉取用户昵称、头像 URL、openID/unionID |
| **必选参数** | `access_token={token}`, `getNickName=1` |
| **所需 scope** | `profile` |
| **代码位置** | `src/backend/src/auth/user_info_api.rs:get_display_info()` |
| **官方文档** | 华为账号开放平台 GOpen.User.getInfo |

### 6. 手机号

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://account.cloud.huawei.com/rest.php?nsp_svc=GOpen.User.getPhone` |
| **方法** | `POST` |
| **Content-Type** | `application/x-www-form-urlencoded` |
| **调用场景** | 拉取用户手机号（中国大陆账号） |
| **必选参数** | `access_token={token}` |
| **所需 scope** | `mobile`（需在 AGC 后台申请） |
| **关键细节** | 响应 body 可能是纯文本（手机号无字段名）或 JSON → 两种格式兼容 |
| **代码位置** | `src/backend/src/auth/user_info_api.rs:get_phone_number()` |
| **官方文档** | 华为账号开放平台 GOpen.User.getPhone |

---

## 二、Drive 网盘 API

### 7. 配额信息

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/about?fields=*` |
| **方法** | `GET` |
| **调用场景** | 每次上传前校验剩余配额（§2.8 第三阶段） |
| **关键细节** | `fields=*` 为**强制**参数（否则 400）；配额字段在 `storageQuota` 子对象下，且华为返回为 **String 类型**（需容忍解析） |
| **响应字段** | `storageQuota.userCapacity`（总容量，String），`storageQuota.usedSpace`（已用，String），`user.displayName`（用户名） |
| **代码位置** | `src/backend/src/drive/about_api.rs:get()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-about-0000001050153641 |

### 8. 列举文件（单页）

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/files?fields=*&pageSize=100&queryParam='{folderId}' in parentFolder` |
| **方法** | `GET` |
| **调用场景** | 浏览目录内容（侧边栏 + 文件列表）；翻页加载更多 |
| **关键细节** | **不用** `parentFolder` 参数！华为只认 `queryParam='id' in parentFolder` 语法。根目录用 `'root'`。单引号必须存在。 |
| **可选参数** | `cursor={nextCursor}`（翻页），`pageSize`（1-500，实测 500 可用） |
| **代码位置** | `src/backend/src/drive/files_api.rs:list()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-fileslist-0000001050153649 |

### 9. 列举文件（全量翻页）

| 项目 | 内容 |
|------|------|
| **API 端点** | 同上（循环调用至 `nextCursor` 为空） |
| **方法** | `GET`（循环） |
| **调用场景** | BFS 构建云端文件树（`refreshCloudTree`）；首次启动全量拉取 |
| **关键细节** | `pageSize=500`，最多 100 页（~50K 文件），超出截断并告警 |
| **代码位置** | `src/backend/src/drive/files_api.rs:list_all()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-fileslist-0000001050153649 |

### 10. 获取单个文件元数据

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/files/{fileId}?fields=*` |
| **方法** | `GET` |
| **调用场景** | 查询单个文件详细信息；upload resume 尾部兜底确认；属性面板 |
| **代码位置** | `src/backend/src/drive/files_api.rs:get()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-filesget-0000001050153637 |

### 11. 创建文件夹

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/files?fields=*` |
| **方法** | `POST` |
| **Content-Type** | `application/json` |
| **调用场景** | 用户点击「新建目录」；同步引擎发现本地有新文件夹 |
| **请求体** | `{ "fileName": "名称", "mimeType": "application/vnd.huawei-apps.folder", "parentFolder": ["{parentId}"] }` |
| **关键细节** | ① `mimeType` 必填（否则 21004001 LACK_OF_PARAM）② root 目录**省略** `parentFolder`（对齐官方文档）③ 中文名称必须用 **ASCII 转义**（`asciiJsonEncode`，否则 21004002 fileName can not be blank）④ 400/409 时检查同名已存在文件夹，存在则视为成功（竞态容错） |
| **代码位置** | `src/backend/src/drive/files_api.rs:create_folder()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-filescreate-0000001050153629 |

### 12. 更新文件（重命名/移动/改描述）

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/files/{fileId}` |
| **方法** | `PATCH` |
| **Content-Type** | `application/json` |
| **调用场景** | 用户重命名文件/文件夹；移动文件到其他目录；改名检测 |
| **请求体** | `{ "fileName": "新名", "parentFolder": ["{newParentId}"], "description": "新描述" }`（仅传变更字段） |
| **关键细节** | 中文名同样必须 `asciiJsonEncode` |
| **代码位置** | `src/backend/src/drive/files_api.rs:update()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-filesupdate-0000001050153633 |

### 13. 删除文件（移入回收站）

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/files/{fileId}` |
| **方法** | `PATCH` |
| **请求体** | `{"recycled": true}` |
| **调用场景** | 用户删除文件；同步引擎发现本地已删且云端存在 → 双向删除 |
| **关键细节** | ⚠️ `DELETE` 方法是**永久删除**（不可恢复），不进回收站。移入回收站（"最近删除"）必须用 `PATCH` 设置 `recycled: true`。回收站文件可通过 `PATCH` 设置 `recycled: false` 恢复。清空回收站用 `DELETE /drive/v1/files/recycle` |
| **代码位置** | `src/drive/files_api.rs:delete()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMS-References/drivekit-server-api-filesupdate |

### 14. 搜索文件

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/files?fields=*&pageSize=100&queryParam=fileName:contains:"{keyword}"` |
| **方法** | `GET` |
| **调用场景** | AppBar 搜索框输入关键词搜索文件和文件夹 |
| **关键细节** | `fileName:contains:"keyword"`；可与 `and '{parentId}' in parentFolder` 叠加限定搜索范围 |
| **代码位置** | `src/backend/src/drive/files_api.rs:search()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-fileslist-0000001050153649 |

### 15. 缩略图

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/thumbnails/{fileId}?form=content` |
| **方法** | `GET` |
| **调用场景** | 文件列表中图片/视频文件显示缩略图（F-UI-05） |
| **响应** | 二进制图片数据 |
| **代码位置** | `src/backend/src/drive/thumbnail_api.rs:get()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-thumbnails-0000001050153621 |

### 16. 增量变更 —— 获取初始游标（getStartCursor）

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/changes/getStartCursor` |
| **方法** | `GET` |
| **调用场景** | 自动云端刷新建立增量基线（全量 BFS 后调用一次，持久化 cursor 供后续增量拉取） |
| **关键细节** | 华为 `/changes` 接口强制要求 cursor，无 cursor 直接 400 "Cursor can't be null"（errorCode `21004001`）。初始 cursor **必须**先通过本端点获取，不能用 `list_changes(None)`（GDrive 风格，华为不支持） |
| **响应字段** | `category`（"drive#startCursor"）、`startCursor`（纯数字字符串，如 "311296"） |
| **cursor 形态** | 递增数字字符串，非 GDrive 的长 token |
| **代码位置** | `src/drive/changes_api.rs:get_start_cursor()` |
| **官方文档** | https://developer.huawei.com/consumer/cn/doc/HMSCore-References-V5/server-api-changesgetstartcursor-0000001050153659-V5 |

**响应示例：**
```json
{
  "category": "drive#startCursor",
  "startCursor": "311296"
}
```

### 17. 增量变更 —— 拉取变更列表（Changes:list）

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/changes?fields=*&pageSize={n}&cursor={cursor}` |
| **方法** | `GET` |
| **必选参数** | `cursor`（**必填且不能为空**，由 getStartCursor 或上一次响应的 newStartCursor 提供） |
| **可选参数** | `fields=*`、`pageSize`（分页大小） |
| **调用场景** | 自动云端刷新的增量路径：用持久化 cursor 拉取自上次以来的文件变更，merge 进内存 cloud_tree |
| **分页** | 响应的 `newStartCursor` 作为下一次请求的 cursor；无更多变更时与请求 cursor 相同 |
| **错误处理** | cursor 无效 → 400 "Cursor is invalid"（`21004002`）；cursor 过期 → 410 "Cursor has expired"（`21084100` CURSOR_EXPIRED）。两种错误都由调用方回退全量 BFS + 清 cursor 重建基线 |
| **代码位置** | `src/drive/changes_api.rs:list_changes()` / `list_all_changes()`（自动分页） |
| **官方文档** | https://developer.huawei.com/consumer/cn/doc/HMSCore-References-V5/server-api-changeslist-0000001050151710-V5 |

**响应示例（含一条删除变更）：**
```json
{
  "category": "drive#changeList",
  "changes": [
    {
      "category": "drive#change",
      "changeType": "trashDone",
      "deleted": false,
      "file": { /* 完整 DriveFile，删除事件也带（recycled: true） */ },
      "fileId": "ADz3nes6G34...",
      "time": "2026-07-06T05:51:13.053Z",
      "type": "File"
    }
  ],
  "newStartCursor": "311298"
}
```

**⚠️ 与 GDrive 协议的关键差异（真机验证，2026-07-06）：**

| 字段/行为 | GDrive 协议 | 华为实际 |
|---|---|---|
| 分页游标字段 | `nextCursor` | **`newStartCursor`** |
| 删除判定 | `removed: true` | **`changeType == "trashDone"`** |
| `deleted` 字段 | 无 | 恒为 `false`（华为用 changeType 区分，不看 deleted） |
| 删除事件 file | 只带 fileId | **带完整 file**（`recycled: true`） |
| 初始 cursor | `changes/getStartPageToken` | **`changes/getStartCursor`** |
| 无 cursor 调用 | 返回首页 | **直接 400**（cursor 必填） |

> **变更类型判定（代码实现，`src/drive/changes_api.rs:Change::from_json`）：**
> - 删除（移入回收站）：`changeType == "trashDone"` 为主判定，`file.recycled == true` 兜底
> - 增/改：`changeType` 非 trashDone（如 update/create，具体值未全部探查，非 trashDone 即按 Modified 处理）

---

## 三、上传 API

### 18. 小文件上传（≤20MB）

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/upload/drive/v1/files?uploadType=multipart` |
| **方法** | `POST` |
| **Content-Type** | `multipart/related; boundary=hwcloud_{timestamp}` |
| **调用场景** | 用户上传文件、同步引擎上传本地新增/修改的文件（≤20MB） |
| **请求体** | Google Drive 风格 multipart/related：第 1 部分 `application/json`（metadata `{fileName, parentFolder?}`），第 2 部分 `application/octet-stream`（文件二进制） |
| **关键细节** | 必须用 `multipart/related`（**不是** `multipart/form-data`，后者华为返回 400）；metadata 用普通 JSON（容忍 UTF-8，不需要 asciiJsonEncode） |
| **代码位置** | `src/backend/src/drive/upload_api.rs:upload_small()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-filescreate-0000001050153629 |

### 19. 大文件分片上传 —— 初始化会话

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/upload/drive/v1/files?uploadType=resume` |
| **方法** | `POST` |
| **调用场景** | 大文件（>20MB）上载第一步：创建 resume 上传会话 |
| **请求头** | `X-Upload-Content-Length: {totalSize}` |
| **请求体** | `{ "fileName": "name", "parentFolder": ["{parentId}"] }`（JSON） |
| **响应 body** | `{"sliceSize": 10485760}` — 仅返回推荐分片大小（10MB），**不含** `serverId`/`uploadId` |
| **★ 响应头 Location** | 会话 URL，格式 `https://driveapis.cloud.huawei.com.cn/upload/drive/v1/{token}/files?uploadType=resume&uploadId={id}`。**后续所有分片 PUT 必须直接使用此 URL**，不能用 `serverId`/`uploadId` 拼接 |
| **代码位置** | `src/drive/upload_api.rs:init_resume_session()` |

### 20. 大文件分片上传 —— 上传分片

| 项目 | 内容 |
|------|------|
| **API 端点** | **直接使用 init 响应 `Location` 头返回的 URL**（不拼接） |
| **方法** | `PUT` |
| **Content-Type** | `application/octet-stream` |
| **调用场景** | 循环上传分片（分片大小 = init 返回的 `sliceSize`，默认 5MB） |
| **请求头** | `Content-Range: bytes {offset}-{end}/{totalSize}`, `Content-Length: {chunkLen}` |
| **请求体** | 二进制分片数据 |
| **★ 中间响应** | **HTTP 308 Resume Incomplete**，body `{"sliceSize":10485760, "rangeList":["0-10485759"], "processTime":8000}`。`rangeList` 最后一段的 `end+1` 即为下一个 offset。**308 是正常响应（非错误）**，不应重试 |
| **最终响应** | HTTP 200/201，body 含完整文件元数据 `{"id", "fileName", "size", ...}`（可能直到最后一片才返回） |
| **重试策略** | 每片 3 次重试（退避 1s/2s/3s），仅在 4xx/5xx 时重试，308 直接前进 |
| **代码位置** | `src/drive/upload_api.rs:put_chunk()` |

### 21. 大文件分片上传 —— 查询最终状态

| 项目 | 内容 |
|------|------|
| **API 端点** | **init 响应 `Location` 头返回的 URL** |
| **方法** | `PUT` |
| **调用场景** | 所有分片发送完毕但未拿到文件元数据时（末片也返回 308），查询上传最终状态 |
| **请求头** | `Content-Range: bytes */{totalSize}`, `Content-Length: 0` |
| **响应** | HTTP 200，body 含完整文件元数据 `{"category":"drive#file","fileName":"...","id":"...","mimeType":"..."}` |
| **关键细节** | `bytes */total` 是 Google Drive 协议的标准上传状态查询方式；需在分片循环结束后调用 |
| **代码位置** | `src/drive/upload_api.rs:upload_resume()` / `upload_resume_with_token()` |

### 22. 上传覆盖已有文件（PATCH）

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/upload/drive/v1/files/{fileId}?uploadType=multipart` |
| **方法** | `PATCH` |
| **Content-Type** | `multipart/related; boundary=hwcloud_{timestamp}` |
| **调用场景** | 冲突解决（local wins）：用本地内容覆盖云端已有文件 |
| **请求体** | 同小文件上传（metadata `{fileName}` + 文件二进制） |
| **回退策略** | PATCH 失败 → DELETE 旧文件 → POST 新建（避免冲突副本循环） |
| **代码位置** | `src/backend/src/drive/upload_api.rs:upload_update()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-filesupdate-0000001050153633 |

---

## 四、下载 API

### 21. 下载文件

| 项目 | 内容 |
|------|------|
| **API 端点** | `https://driveapis.cloud.huawei.com.cn/drive/v1/files/{fileId}?form=content` |
| **方法** | `GET` |
| **调用场景** | 用户下载文件；同步引擎从云端同步文件到本地 |
| **响应** | 流式二进制 |
| **关键细节** | 原子写：先流式写 `<dest>.tmp` → 完成后 `rename` 为 `dest`。错误时清理 `.tmp` 残留（防止 local_watcher 误判为新增文件上传）。 `.tmp` 后缀全链路跳过（watcher/scanner/planner）。 |
| **代码位置** | `src/backend/src/drive/download_api.rs:download()` |
| **官方文档** | https://developer.huawei.com/consumer/en/doc/HMSCore-References/server-api-filesget-0000001050153637 |

---

## 五、API 全局配置

### 域名汇总

| 用途 | 域名 |
|------|------|
| OAuth 认证 | `oauth-login.cloud.huawei.com` |
| 账号信息 | `account.cloud.huawei.com` |
| Drive REST API（CRUD/搜索/缩略图/下载） | `driveapis.cloud.huawei.com.cn/drive/v1` |
| 上传 API（multipart + resume） | `driveapis.cloud.huawei.com.cn/upload/drive/v1` |

### 认证方式

所有 Drive/Upload API 请求均需携带：
```
Authorization: Bearer {access_token}
```

Token 端点（`/oauth2/v3/token`）除外——不注入 auth，否则导致循环刷新。

### 通用请求头

| Header | 值 |
|--------|-----|
| `Authorization` | `Bearer {access_token}` |
| `Content-Type` | `application/json`（除上传用 multipart/related） |
| `Accept` | `application/json` |

### 关键怪癖（踩坑清单）

| # | 怪癖 | 影响 API | 处理方式 |
|---|------|---------|---------|
| 1 | `category` 恒为 `"drive#file"`，类型在 `mimeType` | List/Get | 用 `mimeType` 判断文件夹（`application/vnd.huawei-apps.folder`） |
| 2 | scope `/` 不能 URL 编码 | Authorize | scope 单独拼接，空格 → `%20`，`/` 保留 |
| 3 | authorization_code 含 `+`，form-urlencoded 当空格 | Token | 手工拼 form body + 精确编码 |
| 4 | 中文名 → application/json 报 400 | Create/Update | `asciiJsonEncode` → `\uXXXX` 转义 |
| 5 | 不支持 `parentFolder` 参数，只能用 `queryParam` 语法 | List/Search | `queryParam='id' in parentFolder` |
| 6 | 配额字段为 String 类型 | About | `tolerant_parse_int` 容忍 String |
| 7 | 仅接受 `multipart/related`（拒绝 `multipart/form-data`） | Upload | Google Drive 风格多段 body |
| 8 | DELETE 返回 204 后 LIST 仍返回旧数据（最终一致性） | Delete + List | `_recentlyDeletedPaths` 防振荡守卫 |
| 9 | Root 不是字面 `"root"`，是真实 folder ID | BFS | `_detectRootFolderId` 动态发现 |
| 10 | `serverId` ≠ `fileId`（resume 会话标识 ≠ 文件标识） | Upload Resume | 尾部兜底用 `createdFileId` 查 `/files/{fid}`，不能用 `sid` |
| 11 | 刷新 token 可能不含新 `refresh_token` | Token Refresh | 沿用旧 `refresh_token` |
| 12 | Resume init 仅返回 `{"sliceSize":...}`，不含 `serverId`/`uploadId` | Upload Resume | **从 HTTP `Location` 响应头提取会话 URL**，后续 PUT 直接用该 URL |
| 13 | `/changes` 强制要求 cursor，无 cursor 直接 400；初始 cursor 必须先调 `/changes/getStartCursor`（非 GDrive 的 `list(None)`） | Changes | `get_start_cursor()` 建立基线；cursor 失效（410 CURSOR_EXPIRED）回退全量 BFS |
| 14 | 删除判定用 `changeType == "trashDone"`，**非** GDrive 的 `removed:true`；`deleted` 字段恒 false | Changes | `Change::from_json` 用 changeType 判定 + file.recycled 兜底 |
| 15 | changes 分页游标字段是 `newStartCursor`，**非** GDrive 的 `nextCursor` | Changes | `ChangeListResult::from_json` 优先取 newStartCursor |
| 13 | 分片 PUT 返回 **HTTP 308**（非 200），body 含 `rangeList` | Upload Resume | 308 是正常中间响应（表示分片已接收），解析 `rangeList` 末尾 +1 作为下一 offset，**不重试** |
| 14 | 全部 308 后无文件元数据 | Upload Resume | 向会话 URL 发 `PUT bytes */{total}` 查询最终状态 → 200 + 文件 JSON |
| 12 | OIDC userinfo 常 404 | UserInfo | 静默跳过，不报错 |

---

## 六、API 调用链路梳理

### 登录流程
```
LoginPage ──→ auth/login ──→ buildAuthorizeUrl ──→ 打开浏览器
                                         ──→ OauthServer.start(127.0.0.1:9999)
                                         ──→ waitForCallback
                                         ──→ exchangeCodeForToken ──→ POST /oauth2/v3/token
                                         ──→ tokenStore.save
```

### 同步 cycle
```
FSEvents 变更 / 手动刷新 / 启动恢复
  ──→ run_sync_cycle_inner
  ──→ scanLocal
  ──→ planner.plan (3-way diff)
  ──→ detect_renames
  ──→ filter_anti_oscillation
  ──→ executor.execute_all
       ├── do_upload    ──→ POST /upload/drive/v1/files (multipart or resume)
       ├── do_download  ──→ GET /drive/v1/files/{id}?form=content
       ├── do_create_placeholder ──→ mountManager.createPlaceholder
       ├── do_create_folder      ──→ POST /drive/v1/files
       ├── do_delete_from_cloud  ──→ PATCH /drive/v1/files/{id} {"recycled":true}
       ├── do_delete_from_local  ──→ mountManager.deleteLocal
       └── do_conflict   ──→ ConflictResolver.resolve
                            ├── Cloud wins: download + rename
                            └── Local wins: uploadUpdate (PATCH)
```

### BFS 云端树构建
```
refresh_cloud_tree
  ──→ list_all(root) × 8 并发 ──→ GET /drive/v1/files (loop pages)
  ──→ persist → cloudtree_{escaped}.json
  ──→ load_persisted → 非首次启动秒级加载 (~200ms)
```
