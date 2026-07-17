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

- [x] 新建应用级 Composition Root，集中创建并关闭所有长生命周期对象
- [x] `Main.kt` 接入 AppState/ViewModel，不再持有示例文件和假登录状态
- [x] 创建共享 CoroutineScope，定义应用关闭时的取消顺序
- [x] SQLDelight 首次连接执行 `Schema.create`
- [x] 实现 schema v2→v6 迁移和 `PRAGMA user_version`
- [x] 将 `transfer_queue` 补齐到文档终态字段和索引
- [x] repository 返回受影响行数，CAS 失败抛出明确 stale revision
- [x] `updateRunningProgress` 增加 `WHERE state=Running`
- [x] 实现应用级单例 Logger：console + daily rolling file + 1000 条 ring buffer
- [x] ConfigStore 保存失败不再吞异常；补齐 mountConfigured、skipPatterns、sort 等字段
- [x] NetGuard 暴露 StateFlow、被动失败边沿和 generation 生命周期
- [x] 建立开发/测试数据目录隔离，避免测试污染正式数据

### 必须补充的测试

- [x] 临时 SQLite 文件首次建库测试
- [x] v2/v3/v4/v5 fixture 升级到 v6 的 migration 测试
- [x] CAS 并发与迟到 progress 回调测试
- [x] ConfigStore 原子写、损坏文件和权限失败测试
- [x] Logger 共享缓冲、脱敏和滚动文件测试

### P0 执行记录（2026-07-16）

- `ApplicationRoot` 成为桌面进程唯一装配入口；`Main.kt` 只消费 `DesktopUiState`
- 新库直建 v6，旧库按 v2/v3/v4/v5 fixture 逐级迁移；`PRAGMA user_version=6`
- 传输队列已对齐终态 26 列与 5 个索引，CAS/迟到 progress 测试覆盖
- 配置、DB、token、日志统一从可注入 `AppPaths` 派生；支持 `PETALLINK_DATA_DIR` 与 `PETALLINK_ENV=dev`
- `./gradlew :shared:jvmTest`：185 tests，0 failures，0 errors

### 验收门

- 应用首次启动不报 no such table
- 第二次启动可读取同一配置和 DB
- 所有业务服务由 Composition Root 创建且能被 UI 调用
- 测试不会写入用户真实 Application Support 目录

## 3. 阶段 P1：Drive 数据模型、HTTP 合同与 OAuth

### 目标

先把远端协议修正为可信基础，再允许同步引擎执行任何写操作。

### 3.1 Drive DTO 和严格解析

- [x] 重写 DriveFile，兼容 `fileName/name`、`mimeType`、`parentFolder[]`、`createdTime`、`editedTime`
- [x] size/配额兼容 number、float 和 String
- [x] 内容 hash 兼容 sha256/md5/md5Checksum/fileSha256/hash/contentHash
- [x] 文件夹分类恢复四种完整 MIME 值
- [x] 实现严格 `parse_file_list_page`、`parse_drive_file_strict`、`single_parent`
- [x] 所有已实现写响应核验 HTTP 200、File id、name、parent、size 和 recycled 语义

### 3.2 Files/Changes/About/Thumbnail

- [x] list 使用真实 `'{parentId}' in parentFolder`，root 使用 `'root'`
- [x] query DSL 整体仅编码一次并拒绝引号/反斜线注入
- [x] listAll 检测 cursor 循环和页数上限，不返回部分结果
- [x] 实现 create 前后唯一性核验，处理不确定响应
- [x] 实现 move 的 addParentFolder/removeParentFolder 参数对
- [x] delete 核验 recycled=true，并实现响应丢失 GET 收敛
- [x] thumbnail 改为 `/thumbnails/{id}?form=content`
- [x] Changes 校验 category、fileId、唯一 parent 和终页 newStartCursor
- [x] cursor 400/410 进入全量重建，不推进旧 checkpoint

### 3.3 OAuth 和 token

- [x] 生成 64 字节 verifier、SHA-256 challenge 和 32 字节 state
- [x] 打开系统浏览器后等待 127.0.0.1 loopback 回调
- [x] 校验 state，错误或取消时关闭 listener
- [x] 实现真正 singleflight token refresh
- [x] 三端点并行聚合用户信息，单端失败不阻断
- [x] token 保存采用 tmp + chmod 0600 + atomic rename
- [x] 读取不到 IOPlatformUUID 时 fail closed，不用用户名降级派生密钥
- [x] logout 清 token、内存、DB、云树、同步快照和挂载配置

### 必须补充的测试

- [x] 将原 Rust Drive 测试合同迁移到 Ktor MockEngine/MockWebServer
- [x] 中文和 emoji JSON 的最终 HTTP body 字节测试，防双重转义
- [x] 401 单次重放、写请求不确定结果、Retry-After 日期测试
- [x] OAuth state mismatch、取消、超时和并发 refresh 测试

### P1 执行记录（2026-07-16）

- Files application/json 写请求使用 ASCII-only `\\uXXXX`；multipart metadata 保持原始 UTF-8
- create/rename/move/delete 都实现写后严格核验和响应丢失后的远端 GET/list 收敛，不盲目重放写请求
- OAuth 已接入 PKCE + state + loopback + 取消/超时；用户资料按 OIDC < info < phone 并发聚合
- multipart/related、Location resume URL、308 rangeList、配额 String、Changes 三种删除、两类 cursor 和 OIDC 404 均有自动化合同测试
- `./gradlew :shared:jvmTest`：220 tests，0 failures，0 errors
- P3 已将 `ChangesCursorInvalid(400/410)` 接到“保留旧 checkpoint → startCursor → BFS → replay → 原子提交”，旧 checkpoint 不会被部分结果推进

### 验收门

- Mock HTTP 覆盖文档 18 条华为怪癖
- UI 可以完成真实登录并列出根目录/子目录
- create/rename/move/delete 每项都有响应丢失恢复测试
- 仍不启动自动同步写入，直到 P2/P3 完成

## 4. 阶段 P2：macOS 文件系统平台层

### 目标

提供同步引擎可依赖的真实、本地、安全文件系统抽象。

### 工作项

- [x] 使用 `Files.readAttributes(..., "unix:ino")` 读取 inode
- [x] 实现递归 local scan，输出 path/inode/size/mtime/type/state
- [x] 实现 `local_inode_map` lookup/upsert/purgeMissing
- [x] 实现基于 inode 的 detectMoves
- [x] 通过 JNA 实现 getxattr/setxattr/removexattr
- [x] 实现 PlaceholderManager，严格保护无 state xattr 用户文件
- [x] 实现 FinderInfo 32 字节读改写和灰标清理
- [x] 实现 FSEvents 递归监听、3s debounce、2s warmup、generation
- [x] 实现 lsof 解析、10 进程白名单和 1 秒双重检查
- [x] 实现 64KiB 流式 hasher 与 mtime/size cache
- [x] 所有扫描和监听路径统一执行 `.hwcloud_`/tmp/skipPatterns 过滤

### 必须补充的测试

- [x] 临时目录 scan、rename、copy、delete 和 inode identity 测试
- [x] 0 字节用户文件不得被识别为 placeholder
- [x] modified placeholder 备份测试
- [x] xattr 真机测试和 FSEvents warmup/generation 测试
- [x] lsof busy/whitelist/持续编辑测试

### 验收门

- 本地 rename 保持 fileId，copy 产生新身份
- 用户文件不会因 size 或文件名被误转换、误删除
- FSEvents 不轮询、不吞 warmup 后的新事件、不接收旧 generation 回调

### P2 执行记录（2026-07-16）

- `JvmLocalFileScanner` 已输出 inode、大小、毫秒 mtime、类型、placeholder/state，与 watcher 共用 `SkipFilter`
- `RepositoryInodeIdentityStore` + `InodeMoveDetector` 完成 inode 映射的 lookup/upsert/purge 与 rename/copy 身份判定
- `MacXattrAccess`、`MacFSEventSourceFactory`、`LsofFileBusyChecker` 均走 macOS 真实系统调用；真机 xattr、递归 FSEvents、当前 JVM 文件占用测试通过
- `JvmPlaceholderManager` 仅以 state xattr 判定占位符，保护用户 0 字节文件，支持 modified placeholder 备份、downloaded 标记和 FinderInfo 读改写
- 挂载路径同时接受 macOS `/var` 与 `/private/var` 系统别名，但仍拒绝越界和挂载目录内符号链接
- `JvmFileHasher` 使用 64KiB 流式 SHA-256、per-path mutex、mtime/size cache 和哈希前后稳定性复核
- `./gradlew :shared:jvmTest` 完整回归通过：243 tests，0 failures

## 5. 阶段 P3：云树、Planner 和同步周期闭环

### 目标

完成可信云树、三方 diff、动作执行和状态发布的完整同步周期。

### 5.1 云树 checkpoint

- [x] 全量流程固定为 getStartCursor → BFS → Changes replay
- [x] BFS 并发 8、失败重试不超过 2 次、根目录平局 fail closed
- [x] 实现完整 `validateTrusted`
- [x] tree/pathToId/root/cursor/complete 单文件 checkpoint
- [x] tmp fsync → bak → rename → parent fsync，失败恢复旧 checkpoint
- [x] 增量先应用到 clone，成功后原子提交
- [x] rename/move rekey 整个子树，删除移除整个子树
- [x] 连续 300 次增量强制全量

### 5.2 Planner/Executor

- [x] 用真实 editedTime 修正 24 种 planner 决策
- [x] 实现不可信删除守卫、pending 收敛和启动恢复守卫
- [x] 实现目录保护和救援 CreateFolder
- [x] 两阶段目录优先并在阶段间回填 parentFileId
- [x] 上传前执行完整稳定性检查与 `[0,2,3,5]` 重试窗口
- [x] 每个 ActionResult 与原 action 按索引严格对应

### 5.3 状态和周期所有权

- [x] `CycleCoordinator` 成为唯一同步周期 owner
- [x] watcher/manual/timer/startup/recovery 只提交 CycleRequest
- [x] ActivityTracker shutdown 后拒绝新动作并等待已登记动作
- [x] StatusAggregator 发布完整快照和单调 revision
- [x] 建立 StateFlow/SharedFlow：sync state、folder change、transfer update、upload failed

### 验收门

- 空云盘也能形成可信 checkpoint
- 中途任一页失败不会提交部分云树或推进 cursor
- 无可信云树时绝不执行本地/云端删除
- 临时目录 + Mock Drive 能完成一个完整双向同步周期

### P3 执行记录（2026-07-16）

- `BfsCloudTreeRefresher` 与 `JvmCloudTreeCheckpointStore` 已实现可信全量/增量云树、cursor 失效回退、子树 rekey/delete、300 次强制全量和原子 checkpoint 回滚。
- Planner/Executor 已接入真实 `editedTime`、目录保护与救援、两阶段建目录、`parentFileId` 回填、上传稳定性窗口和严格结果索引。
- `CycleRequestDispatcher` 统一接收 startup/watcher/manual/timer/recovery 请求；`ActivityTracker`、完整状态快照和事件流已有并发测试覆盖。
- `JvmSyncRuntime` 已装配真实本地扫描、Mock Drive 云树/上传/占位、数据库 baseline 和同一可信 checkpoint；临时目录纵向测试验证本地新增上传与云端新增落地。
- P3 收口时，覆盖上传、完整下载、破坏性删除、移动和冲突动作曾保持显式延后；该安全门已在 P4 合同与回归完成后解除。
- `./gradlew :shared:jvmTest --rerun-tasks` 完整回归通过：271 tests，0 failures。

## 6. 阶段 P4：TaskRunner、安全传输和释放空间

### 目标

恢复原项目的数据安全合同，使网络异常、崩溃和重启不会导致重复创建或误删。

### 6.1 九态 TaskRunner

- [x] 所有迁移必须先通过 canTransition，非法迁移直接失败
- [x] CAS 失败不继续执行结算
- [x] 补齐 ColumnPatch Keep/Set/Clear 的 SQL 语义
- [x] 实现 next_retry_at 和带 jitter 的 1/2/4/8/16 秒退避
- [x] Running 上传重启进入 VerifyingRemote，不直接 Failed
- [x] 实现固定启动恢复和在线恢复顺序
- [x] Completed/Canceled 保持无出边；Failed 仅通过显式 retry 重规划

### 6.2 上传

- [x] ≤20MiB multipart/related Create/Update
- [x] >20MiB resume init、分片 PUT、308 rangeList 和最终查询
- [x] session URL、offset 和源文件 snapshot 持久化
- [x] Update 永不降级 Create
- [x] 不确定写只查询服务端确认，不用 offset+chunkLen 推算

### 6.3 下载

- [x] `.tmp` + `.download-meta.tmp` 持久化
- [x] Range/206 Content-Range 严格核验
- [x] 416 仅允许一次从 0 重启
- [x] 1MiB buffer 流式 SHA-256
- [x] 安装前二次 fetch metadata
- [x] 文件和父目录 fsync 后 atomic rename
- [x] 暂态错误保留断点，永久错误清理断点

### 6.4 释放空间

- [x] 实现完整 13 步 TOCTOU 流程
- [x] staging 文件与 free_up_staging DB 记录同事务协调
- [x] 远端核验夹在两次本地/DB snapshot 复核之间
- [x] commit 后创建占位并更新 inode 映射
- [x] 崩溃启动时恢复或清理中断 staging
- [x] 禁止直接 `Files.deleteIfExists` 作为释放空间实现

### 验收门

- 传输在每个持久态崩溃重启后都能安全恢复
- 网络响应丢失不会重复创建远端文件
- 下载不会把两个远端版本拼成一个本地文件
- 释放空间压力测试不误删正在编辑或身份已变化的文件

### P4 执行记录（2026-07-16）

- `TransferRepository.transition` 使用 revision CAS 和完整 `TransferPatch`，实现 nullable 字段 Keep/Set/Clear、Running revision 进度保护和持久 resume 会话；TaskRunner 执行真实 `next_retry_at`、1/2/4/8/16 秒+jitter、显式 retry 与固定恢复顺序。
- ≤20MiB Create/Update 与 >20MiB resume 均进入同一 TaskRunner；分片只接受 308 `rangeList` 或状态查询确认的 offset，响应丢失通过 persisted id 或 parent/name/size/时间窗/content hash 只读收敛。
- `JvmTransferFileStore` 实现 `.tmp`/sidecar、Range/206、416 单次回退、1MiB 流式 SHA-256、二次远端版本核验、文件/目录 fsync 和原子安装；三个下载/上传命令不再绕过持久协议。
- `JvmFreeUpService` 用 write-ahead `free_up_staging`、双重本地/DB 快照、可信 checkpoint、远端复核、原子 staging、占位符、baseline CAS 和 inode 更新完成释放空间；启动恢复绝不覆盖新用户文件。
- 同步运行时已解除 P3 的破坏性动作安全门，云端删除、本地安全删除、移动/重命名和冲突副本会同步收敛 checkpoint 与 baseline。
- `./gradlew :shared:jvmTest --rerun-tasks` 完整回归通过：291 tests，0 failures。

## 7. 阶段 P5：Compose UI 与 macOS 生命周期

### 目标

将已验证业务闭环暴露为完整桌面产品，并恢复后台常驻体验。

### 7.1 UI 与 ViewModel

- [x] Splash/恢复登录/登录/Main/Settings/Logs 完整路由
- [x] AuthViewModel 接入真实登录、取消、错误和用户信息
- [x] FileBrowserViewModel：分页、面包屑、排序、搜索、目录树
- [x] SyncViewModel 拒绝旧 revision，同 revision 幂等
- [x] TransferViewModel 同时执行 requestId 与 per-task revision 保护
- [x] 文件列表六列、缩略图、多选和批量操作
- [x] 右键菜单视口钳制、异步 canFreeUp 和确认 Dialog
- [x] 设置页加载/保存真实配置并处理挂载目录切换
- [x] 日志页共享 ring buffer、导出和清除
- [x] 深色模式、Reduced Motion 和 27 个 Mate 组件验收

### 7.2 macOS 生命周期

- [x] 单实例锁与已运行实例聚焦
- [x] NSStatusItem 菜单及 5 秒重建节流
- [x] Close/Cmd+Q/Dock Quit 隐藏并切 accessory
- [x] 托盘 show 时切回 regular 并聚焦
- [x] Apple Event 区分系统关机并放行真实退出
- [x] LaunchAgent enable/disable/status 和 `--hidden`
- [x] 优雅关闭等待活动、flush checkpoint，并设置 incomplete 哨兵

### 验收门

- 所有原 49 个业务命令都有真实 ViewModel 使用路径或明确替代
- 关窗后同步继续；第二实例不会启动第二个 watcher
- 设置、日志、传输、目录树状态在重启后保持一致

### P5 执行记录（2026-07-16）

- `ApplicationRoot`/`DesktopAppViewModel` 成为 UI 唯一业务入口：恢复登录、OAuth 取消与错误、用户信息、六分区设置、日志 ring buffer、文件操作和持久传输重试均调用真实 `CommandService`；重复命令采用安全替代路径（普通下载统一走按需下载 TaskRunner，传输历史保留 completed/failed/finished 三种精确语义且 UI 默认 finished，单项释放统一由 batch 协议执行）。
- `FileBrowserViewModel` 完成 requestId 分页保护、面包屑、服务端搜索、文件夹优先排序和已加载 children 目录树；文件区提供 checkbox/name/size/time/status/actions 六列、缩略图、多选批量下载/释放/删除、右键异步 `canFreeUp`、窗口边界钳制与确认对话框。
- `SyncViewModel` 拒绝 `revision <= lastRevision`；`TransferViewModel` 同时拒绝旧 requestId 和低于现有 task revision 的列表/进度回写，并已接入桌面状态流。
- Compose SystemTray（macOS 落为 NSStatusItem）显示动态同步/传输菜单并按 5 秒节流；文件锁+loopback 保证单实例，Close/Cmd+Q/Dock Quit 隐藏并切 accessory，托盘/第二实例切 regular 并聚焦。
- JNA 检测 `NSAppleEventManager.currentAppleEvent` 的 `aevt/quit` 以放行系统关机；LaunchAgent 原子写入、bootstrap/bootout 和 `--hidden` 已实现；退出以 ActivityTracker 封门、3.2 秒等待和 `incomplete-shutdown` 哨兵保护可信 checkpoint。
- 系统深色主题、Reduced Motion CompositionLocal 与完整 Mate 组件目录已接入；全量 `./gradlew :shared:jvmTest --rerun-tasks` 通过：296 tests，0 failures，`git diff --check` 通过。

## 8. 阶段 P6：更新、打包和兼容验收

### 工作项

- [x] 统一版本来源并替换旧 Tauri release 规则
- [x] 配置应用图标、bundle id、entitlements 和最低 macOS 版本
- [x] 选择并实现 Sparkle 或等价更新方案
- [x] 建立签名、公证、staple 和 DMG 发布流程
- [x] 更新前等待传输空闲，最多 5 分钟
- [x] 建立 CI：单测、合同测试、集成测试、DMG 构建
- [x] 迁移旧 token/config/DB 或明确提供安全重建策略
- [ ] 与原 Tauri 版本执行功能对照验收

### P6 自动化执行记录（2026-07-16）

- 根目录 `petalLinkVersion=1.0.12` 是 UI、命令、包版本和 tag 校验的唯一版本来源；旧 Tauri release 规则已替换为 CMP Desktop 规则。
- 沿用原 bundle id `io.github.yuanbaobaoo.PetalLink` 和原图标，最低 macOS 12.0；entitlements 支持 JVM/JNA native library，jlink 显式包含 SQLite JDBC 所需 `java.sql`。
- 等价更新器实现启动 3 秒静默检查、每小时检查、聚焦 10 分钟节流、手动检查和主界面提示；安装链路包含 HTTPS manifest、语义版本、最多 5 分钟传输空闲等待、流式下载、SHA-256、codesign/Gatekeeper/固定 Team ID 校验、helper 替换及失败回滚，无 Team ID 时 fail closed。
- CI 执行完整测试、DMG 构建、静态制品门禁以及打包应用隐藏启动/单实例冒烟；Release workflow 执行 Developer ID 签名、notary、staple、Gatekeeper、更新 zip/manifest 与 GitHub Release。
- 旧数据目录直接沿用；原 Tauri `token.bin` 明文布局、camelCase 配置枚举和数据库 v2–v6 均有兼容测试。49 命令源码审计补齐了挂载切换清基线、账号隔离、直接 rename/move/delete 结算、后台目录 BFS、完整状态快照、传输顺序和批量释放统计。
- `./gradlew :shared:jvmTest`：317 tests，0 failures；`packageDmg` 实际生成 arm64 `PetalLink-1.0.12.dmg`；最新 `.app` 的制品门禁、隐藏启动/第二实例冒烟通过。DMG 挂载后复制到隔离安装目录的流程已在前一构建通过，最新哈希尚待重复该步。
- 代码、无签名构建和自动化门禁已经完成；正式 Developer ID/notarization、真实华为账号与完整系统 UI 对照仍需在 Release Candidate 上按 `13-发布与兼容验收.md` 人工签字，因此 P6 尚未整体关闭。

### P6 追加执行记录（2026-07-17）：bundle id dev/release 分离

- **问题**：原实现打包的 `CFBundleIdentifier` 硬编码为 prod，与运行时 `PETALLINK_ENV` 数据目录开关完全解耦——dev 包会误读 prod 数据目录，dev/release 开机自启 LaunchAgent 互相覆盖。
- **方案（单一真相源）**：新增 gradle 属性 `petalLinkBuildProfile`（默认 release）。打包期同时写入 `.app` 的 `CFBundleIdentifier` 和编译进 `BuildInfo.BUNDLE_ID`/`BUILD_PROFILE`；运行时 `AppPaths.resolveFromEnvironment` 默认读 `BuildInfo.BUNDLE_ID` 派生数据目录。两个开关合一。
- **LaunchAgent**：`CommandService.launchAgentManager()` 改用 `AppPaths.currentBundleId()`，dev 包注册 `...-dev.plist`，release 包注册 prod plist，互不覆盖。
- **优先级**：`PETALLINK_DATA_DIR` > `PETALLINK_ENV=dev` > `BuildInfo.BUNDLE_ID` > prod 兜底；前两者保留为测试/本地覆盖。
- **顺手清理**：全局修正包名拼写 `yuanbaobaao`→`yuanbaobaoo`（178 文件 + 5 目录）；删除零引用死代码 `core/Paths.kt`（其 `cacheBaseDir` 用了游离的 `Application Support/PetalLink` 路径，与 bundle id 体系冲突，运行时实际走 `AppPaths.cloudTreeCheckpoint`）。
- **测试**：新增 `AppPathsTest`（优先级链、dev/prod 目录、大小写、空白覆盖，纯函数 `resolveFromEnvironment` 不污染全局）；`DesktopLifecycleTest` 补 dev/prod LaunchAgent 隔离测试；修复一个既有 flaky 时序测试（`JvmSyncRuntimeIntegrationTest` 文件落地后未等 `folderSyncProgress` 发布）。`./gradlew :shared:jvmTest --rerun-tasks`：330 tests，0 failures，连续两次稳定。
- **双包验收**：release 包 `CFBundleIdentifier=io.github.yuanbaobaoo.PetalLink`、dev 包 `=...PetalLink-dev`，`BuildInfo` 一致；两包 verify+smoke 通过；release DMG ditto 隔离 verify+冒烟通过；release DMG SHA-256 `3187f02d5e04612dcdd3d6cf16a8e051499b9622f4a61d10522bdc089dfa2249`。
- **已修复 jpackage DMG entitlements 丢失**：jpackage `--type dmg` 在 adhoc 签名下会丢主可执行 entitlements。新增 `repackDmgForEntitlements` 任务，`packageDmg` 后用 `ditto`+`hdiutil` 从 app-image 重封 DMG，dev/release 双包 entitlements 均完整保留。ditto 复制品的 verify 门禁（含 entitlements）现全通过。

### 最终验收矩阵

以下均为 Release Candidate 人工矩阵；自动测试或本地无签名包通过不替代真实账号、签名与系统事件验收。

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
