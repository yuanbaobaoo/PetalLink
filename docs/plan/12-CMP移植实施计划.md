# 12 · CMP 完整移植实施计划

> 本计划以 `11-当前实现审计.md` 为起点。阶段按依赖顺序排列；上一阶段验收门未通过，不进入依赖它的破坏性功能实现。

## 1. 总体交付路线

```text
P0 装配与数据底座
  → P1 Drive/OAuth 合同
    → P2 本地文件系统平台层
      → P3 云树与同步引擎闭环
        → P4 安全传输与释放空间
          → P5 UI 与 macOS 生命周期
            → P6 发布与兼容验收
```

首个关键里程碑不是“页面完成”，而是以下最小纵向闭环：

```text
启动应用
→ 恢复/完成 OAuth
→ 列出真实云端目录
→ 用户选择空挂载目录
→ 建立可信云树 checkpoint
→ 扫描本地
→ 执行一个安全同步动作
→ 重启后无重复上传或误删除
```

## 2. 阶段 P0：应用装配、数据库与可观察性

### 目标

让应用从静态 Demo 变成真实服务容器，确保后续功能有唯一生命周期和持久化底座。

### 工作项

- [ ] 新建应用级 Composition Root，集中创建并关闭所有长生命周期对象
- [ ] `Main.kt` 接入 AppState/ViewModel，不再持有示例文件和假登录状态
- [ ] 创建共享 CoroutineScope，定义应用关闭时的取消顺序
- [ ] SQLDelight 首次连接执行 `Schema.create`
- [ ] 实现 schema v2→v6 迁移和 `PRAGMA user_version`
- [ ] 将 `transfer_queue` 补齐到文档终态字段和索引
- [ ] repository 返回受影响行数，CAS 失败抛出明确 stale revision
- [ ] `updateRunningProgress` 增加 `WHERE state=Running`
- [ ] 实现应用级单例 Logger：console + daily rolling file + 1000 条 ring buffer
- [ ] ConfigStore 保存失败不再吞异常；补齐 mountConfigured、skipPatterns、sort 等字段
- [ ] NetGuard 暴露 StateFlow、被动失败边沿和 generation 生命周期
- [ ] 建立开发/测试数据目录隔离，避免测试污染正式数据

### 必须补充的测试

- [ ] 临时 SQLite 文件首次建库测试
- [ ] v2/v3/v4/v5 fixture 升级到 v6 的 migration 测试
- [ ] CAS 并发与迟到 progress 回调测试
- [ ] ConfigStore 原子写、损坏文件和权限失败测试
- [ ] Logger 共享缓冲、脱敏和滚动文件测试

### 验收门

- 应用首次启动不报 no such table
- 第二次启动可读取同一配置和 DB
- 所有业务服务由 Composition Root 创建且能被 UI 调用
- 测试不会写入用户真实 Application Support 目录

## 3. 阶段 P1：Drive 数据模型、HTTP 合同与 OAuth

### 目标

先把远端协议修正为可信基础，再允许同步引擎执行任何写操作。

### 3.1 Drive DTO 和严格解析

- [ ] 重写 DriveFile，兼容 `fileName/name`、`mimeType`、`parentFolder[]`、`createdTime`、`editedTime`
- [ ] size/配额兼容 number、float 和 String
- [ ] 内容 hash 兼容 sha256/md5/md5Checksum/fileSha256/hash/contentHash
- [ ] 文件夹分类恢复四种完整 MIME 值
- [ ] 实现严格 `parse_file_list_page`、`parse_drive_file_strict`、`single_parent`
- [ ] 所有写响应核验 HTTP 200、File id、name、parent、size 和 recycled 语义

### 3.2 Files/Changes/About/Thumbnail

- [ ] list 使用真实 `'{parentId}' in parentFolder`，root 使用 `'root'`
- [ ] query DSL 整体仅编码一次并拒绝引号/反斜线注入
- [ ] listAll 检测 cursor 循环和页数上限，不返回部分结果
- [ ] 实现 create 前后唯一性核验，处理不确定响应
- [ ] 实现 move 的 addParentFolder/removeParentFolder 参数对
- [ ] delete 核验 recycled=true，并实现响应丢失 GET 收敛
- [ ] thumbnail 改为 `/thumbnails/{id}?form=content`
- [ ] Changes 校验 category、fileId、唯一 parent 和终页 newStartCursor
- [ ] cursor 400/410 进入全量重建，不推进旧 checkpoint

### 3.3 OAuth 和 token

- [ ] 生成 64 字节 verifier、SHA-256 challenge 和 32 字节 state
- [ ] 打开系统浏览器后等待 127.0.0.1 loopback 回调
- [ ] 校验 state，错误或取消时关闭 listener
- [ ] 实现真正 singleflight token refresh
- [ ] 三端点并行聚合用户信息，单端失败不阻断
- [ ] token 保存采用 tmp + chmod 0600 + atomic rename
- [ ] 读取不到 IOPlatformUUID 时 fail closed，不用用户名降级派生密钥
- [ ] logout 清 token、内存、DB、云树、同步快照和挂载配置

### 必须补充的测试

- [ ] 将原 Rust Drive 测试合同迁移到 Ktor MockEngine/MockWebServer
- [ ] 中文和 emoji JSON 的最终 HTTP body 字节测试，防双重转义
- [ ] 401 单次重放、写请求不确定结果、Retry-After 日期测试
- [ ] OAuth state mismatch、取消、超时和并发 refresh 测试

### 验收门

- Mock HTTP 覆盖文档 18 条华为怪癖
- UI 可以完成真实登录并列出根目录/子目录
- create/rename/move/delete 每项都有响应丢失恢复测试
- 仍不启动自动同步写入，直到 P2/P3 完成

## 4. 阶段 P2：macOS 文件系统平台层

### 目标

提供同步引擎可依赖的真实、本地、安全文件系统抽象。

### 工作项

- [ ] 使用 `Files.readAttributes(..., "unix:ino")` 读取 inode
- [ ] 实现递归 local scan，输出 path/inode/size/mtime/type/state
- [ ] 实现 `local_inode_map` lookup/upsert/purgeMissing
- [ ] 实现基于 inode 的 detectMoves
- [ ] 通过 JNA/JNI 实现 getxattr/setxattr/removexattr
- [ ] 实现 PlaceholderManager，严格保护无 state xattr 用户文件
- [ ] 实现 FinderInfo 32 字节读改写和灰标清理
- [ ] 实现 FSEvents 递归监听、3s debounce、2s warmup、generation
- [ ] 实现 lsof 解析、10 进程白名单和 1 秒双重检查
- [ ] 实现 64KB 流式 hasher 与 mtime/size cache
- [ ] 所有扫描和监听路径统一执行 `.hwcloud_`/tmp/skipPatterns 过滤

### 必须补充的测试

- [ ] 临时目录 scan、rename、copy、delete 和 inode identity 测试
- [ ] 0 字节用户文件不得被识别为 placeholder
- [ ] modified placeholder 备份测试
- [ ] xattr 真机测试和 FSEvents warmup/generation 测试
- [ ] lsof busy/whitelist/持续编辑测试

### 验收门

- 本地 rename 保持 fileId，copy 产生新身份
- 用户文件不会因 size 或文件名被误转换、误删除
- FSEvents 不轮询、不吞 warmup 后的新事件、不接收旧 generation 回调

## 5. 阶段 P3：云树、Planner 和同步周期闭环

### 目标

完成可信云树、三方 diff、动作执行和状态发布的完整同步周期。

### 5.1 云树 checkpoint

- [ ] 全量流程固定为 getStartCursor → BFS → Changes replay
- [ ] BFS 并发 8、失败重试不超过 2 次、根目录平局 fail closed
- [ ] 实现完整 `validateTrusted`
- [ ] tree/pathToId/root/cursor/complete 单文件 checkpoint
- [ ] tmp fsync → bak → rename → parent fsync，失败恢复旧 checkpoint
- [ ] 增量先应用到 clone，成功后原子提交
- [ ] rename/move rekey 整个子树，删除移除整个子树
- [ ] 连续 300 次增量强制全量

### 5.2 Planner/Executor

- [ ] 用真实 editedTime 修正 24 种 planner 决策
- [ ] 实现不可信删除守卫、pending 收敛和启动恢复守卫
- [ ] 实现目录保护和救援 CreateFolder
- [ ] 两阶段目录优先并在阶段间回填 parentFileId
- [ ] 上传前执行完整稳定性检查与 `[0,2,3,5]` 重试窗口
- [ ] 每个 ActionResult 与原 action 按索引严格对应

### 5.3 状态和周期所有权

- [ ] `CycleCoordinator` 成为唯一同步周期 owner
- [ ] watcher/manual/timer/startup/recovery 只提交 CycleRequest
- [ ] ActivityTracker shutdown 后拒绝新动作并等待已登记动作
- [ ] StatusAggregator 发布完整快照和单调 revision
- [ ] 建立 StateFlow/SharedFlow：sync state、folder change、transfer update、upload failed

### 验收门

- 空云盘也能形成可信 checkpoint
- 中途任一页失败不会提交部分云树或推进 cursor
- 无可信云树时绝不执行本地/云端删除
- 临时目录 + Mock Drive 能完成一个完整双向同步周期

## 6. 阶段 P4：TaskRunner、安全传输和释放空间

### 目标

恢复原项目的数据安全合同，使网络异常、崩溃和重启不会导致重复创建或误删。

### 6.1 九态 TaskRunner

- [ ] 所有迁移必须先通过 canTransition，非法迁移直接失败
- [ ] CAS 失败不继续执行结算
- [ ] 补齐 ColumnPatch Keep/Set/Clear 的 SQL 语义
- [ ] 实现 next_retry_at 和带 jitter 的 1/2/4/8/16 秒退避
- [ ] Running 上传重启进入 VerifyingRemote，不直接 Failed
- [ ] 实现固定启动恢复和在线恢复顺序
- [ ] Completed/Canceled 保持无出边；Failed 仅通过显式 retry 重规划

### 6.2 上传

- [ ] ≤20MiB multipart/related Create/Update
- [ ] >20MiB resume init、分片 PUT、308 rangeList 和最终查询
- [ ] session URL、offset 和源文件 snapshot 持久化
- [ ] Update 永不降级 Create
- [ ] 不确定写只查询服务端确认，不用 offset+chunkLen 推算

### 6.3 下载

- [ ] `.tmp` + `.download-meta.tmp` 持久化
- [ ] Range/206 Content-Range 严格核验
- [ ] 416 仅允许一次从 0 重启
- [ ] 1MiB buffer 流式 SHA-256
- [ ] 安装前二次 fetch metadata
- [ ] 文件和父目录 fsync 后 atomic rename
- [ ] 暂态错误保留断点，永久错误清理断点

### 6.4 释放空间

- [ ] 实现完整 13 步 TOCTOU 流程
- [ ] staging 文件与 free_up_staging DB 记录同事务协调
- [ ] 远端核验夹在两次本地/DB snapshot 复核之间
- [ ] commit 后创建占位并更新 inode 映射
- [ ] 崩溃启动时恢复或清理中断 staging
- [ ] 禁止直接 `Files.deleteIfExists` 作为释放空间实现

### 验收门

- 传输在每个持久态崩溃重启后都能安全恢复
- 网络响应丢失不会重复创建远端文件
- 下载不会把两个远端版本拼成一个本地文件
- 释放空间压力测试不误删正在编辑或身份已变化的文件

## 7. 阶段 P5：Compose UI 与 macOS 生命周期

### 目标

将已验证业务闭环暴露为完整桌面产品，并恢复后台常驻体验。

### 7.1 UI 与 ViewModel

- [ ] Splash/恢复登录/登录/Main/Settings/Logs 完整路由
- [ ] AuthViewModel 接入真实登录、取消、错误和用户信息
- [ ] FileBrowserViewModel：分页、面包屑、排序、搜索、目录树
- [ ] SyncViewModel 拒绝旧 revision，同 revision 幂等
- [ ] TransferViewModel 同时执行 requestId 与 per-task revision 保护
- [ ] 文件列表六列、缩略图、多选和批量操作
- [ ] 右键菜单视口钳制、异步 canFreeUp 和确认 Dialog
- [ ] 设置页加载/保存真实配置并处理挂载目录切换
- [ ] 日志页共享 ring buffer、导出和清除
- [ ] 深色模式、Reduced Motion 和 27 个 Mate 组件验收

### 7.2 macOS 生命周期

- [ ] 单实例锁与已运行实例聚焦
- [ ] NSStatusItem 菜单及 5 秒重建节流
- [ ] Close/Cmd+Q/Dock Quit 隐藏并切 accessory
- [ ] 托盘 show 时切回 regular 并聚焦
- [ ] Apple Event 区分系统关机并放行真实退出
- [ ] LaunchAgent enable/disable/status 和 `--hidden`
- [ ] 优雅关闭等待活动、flush checkpoint，并设置 incomplete 哨兵

### 验收门

- 所有原 49 个业务命令都有真实 ViewModel 使用路径或明确替代
- 关窗后同步继续；第二实例不会启动第二个 watcher
- 设置、日志、传输、目录树状态在重启后保持一致

## 8. 阶段 P6：更新、打包和兼容验收

### 工作项

- [ ] 统一版本来源并替换旧 Tauri release 规则
- [ ] 配置应用图标、bundle id、entitlements 和最低 macOS 版本
- [ ] 选择并实现 Sparkle 或等价更新方案
- [ ] 建立签名、公证、staple 和 DMG 发布流程
- [ ] 更新前等待传输空闲，最多 5 分钟
- [ ] 建立 CI：单测、合同测试、集成测试、DMG 构建
- [ ] 迁移旧 token/config/DB 或明确提供安全重建策略
- [ ] 与原 Tauri 版本执行功能对照验收

### 最终验收矩阵

- [ ] 新用户：安装 → 登录 → 选择目录 → 首次索引 → 同步
- [ ] 老用户：保留 token/config/DB 或获得明确无损迁移流程
- [ ] 网络：离线启动、传输中断、限流、5xx、401、恢复
- [ ] 文件：空文件、大文件、中文、emoji、重命名、移动、复制、持续编辑
- [ ] 云端：rename/move/delete、cursor 失效、空中间页、完整空盘
- [ ] 生命周期：关窗、Cmd+Q、Dock Quit、系统关机、开机自启、第二实例
- [ ] 发布：DMG 安装、签名验证、公证验证、更新与回滚

## 9. 计划维护规则

- 每完成一个工作项，将复选框改为 `[x]`，并附对应测试或代码入口。
- 发现文档合同与原源码不一致时，先核对原仓库，再修正文档和测试。
- 禁止以 mock 集成测试代替真实装配验收。
- 禁止在 P3 信任门和 P4 安全合同完成前启用自动破坏性同步。
- 每个阶段结束后重新执行一次 `11-当前实现审计.md`，更新完成度和下一阶段阻断项。

