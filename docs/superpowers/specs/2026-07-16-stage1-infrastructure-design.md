# 阶段 1 设计：基础设施层（config / logging / net_guard / error / data）

> 日期：2026-07-16
> 状态：待评审
> 关联文档：docs/01 §技术栈、docs/04 §数据模型、docs/06 §网络守卫、docs/10 阶段 1

---

## 一、目标与范围

补全 PetalLink Kuikly 重构的**基础设施层**（docs/10 阶段 1，无外部依赖）。
这是后续所有阶段（华为 API / 挂载 / 同步引擎 / 平台 / UI）的基石。

5 个模块：

| 模块 | 职责 | 当前骨架状态 | 本阶段交付 |
|---|---|---|---|
| `config` | 配置加载、校验、持久化 | 仅有 AppConfig 常量 | + ConfigStore 接口 + ConfigValidator |
| `logging` | 三层日志（stdout/文件/环形缓冲） | 空 | 接口 + LogLevel + LogRecord + 实际实现 |
| `net_guard` | 网络连通性探测与状态机 | 空 | 接口 + NetState + 探测逻辑 |
| `error` | 错误模型 | 已有 AppError sealed class | + 扁平序列化 + 恢复元数据 |
| `data` | SQLite 持久化 | 已有 DDL + Models | + SQLDelight .sq + repository + CAS + migrations |

### 不在本阶段（明确边界）

- 华为 API 调用、OAuth 流程（阶段 2）
- 文件系统操作、FSEvents（阶段 3）
- 同步引擎、状态机执行（阶段 4）
- 托盘、激活策略、自启动（阶段 5）
- UI 页面、ViewModel（阶段 6）

---

## 二、模块设计

### 2.1 config（配置加载与校验）

**已有**：`AppConfig.kt`（常量：warmup=2s、debounce=3s、poll=60s、concurrency=6、退避参数等）。

**新增**：

```
commonMain/.../config/
├── AppConfig.kt            # 已有：常量（不动）
├── UserConfig.kt           # 新增：用户可配置项 data class
├── ConfigStore.kt          # 新增：expect 配置读写接口
└── ConfigValidator.kt      # 新增：校验规则
```

**`UserConfig`**（用户可调项，对标 src/core/config.rs）：
- `mountDir: String` — 挂载目录
- `concurrency: Int` — 并发数（默认 6）
- `pollIntervalSec: Long` — 轮询间隔（默认 60）
- `debounceSec: Long` — 去抖（默认 3）
- `oauthCallbackPort: Int` — OAuth 回调端口

**`ConfigValidator.validate(config): List<ConfigError>`** — 精确校验规则（对标 docs/10 阶段 1 item 1）：
- `concurrency` ∈ [1, 20]，默认 6
- `pollIntervalSec` == 0（禁用）或 >= 60
- `debounceSec` >= 1
- `oauthCallbackPort` > 0
- `mountDir` 规则：非空、非根目录、`~` 展开为 home、不含 `..`

**`ConfigStore`**（expect/actual）：
- `commonMain`：`expect class ConfigStore { fun load(): UserConfig?; fun save(config: UserConfig) }`
- `macosMain`：actual 用 JSON 文件持久化（路径经 cache_paths 规则）

### 2.2 logging（三层日志）

**设计**：对标 src/core/logging.rs 三层结构。

```
commonMain/.../core/logging/
├── LogLevel.kt         # enum: TRACE/DEBUG/INFO/WARN/ERROR
├── LogRecord.kt        # data class: timestamp, level, target, message, throwable?
├── Logger.kt           # expect: 日志门面（各 level 方法）
└── LogAppender.kt      # 接口：日志输出后端（commonMain 定义，macosMain 实现）

macosMain/.../core/logging/
├── Logger.kt           # actual：路由到各 appender
├── ConsoleAppender.kt  # stdout（格式化输出）
├── FileAppender.kt     # 滚动文件（PetalLink.log，按日轮转保留 30 天）
└── RingBufferAppender.kt  # 环形缓冲（MAX_BUFFER_SIZE=1000，newest-first，供日志查看页）
```

**关键约束**：
- 默认级别 INFO；debug 模式也用 INFO（非 DEBUG，对标原项目）
- EnvFilter 过滤
- **token/secret 绝不打印**（coding-rules.md 硬约束）——Logger 提供 `redact()` 钩子，敏感字段自动脱敏
- 环形缓冲 newest-first，容量 1000，供阶段 6 日志查看页读取

### 2.3 net_guard（网络守卫）

**设计**：对标 src/core/net_guard.rs。

```
commonMain/.../core/net_guard/
├── NetState.kt         # enum: ONLINE / OFFLINE
├── NetGuard.kt         # 接口：探测 + 状态查询 + 订阅
└── NetGuardEngine.kt   # 探测逻辑（纯逻辑，可单测）

macosMain/.../core/net_guard/
└── NetGuard.kt         # actual：TCP 探测实现
```

**精确参数**（docs/06 §网络守卫）：
- 探测目标：`driveapis.cloud.huawei.com.cn:443`，TCP 连接
- 超时：3s
- 间隔：30s
- **防抖**：失败立即转 OFFLINE；**连续 2 次成功**才转 ONLINE
- ProbeLifecycle 代际管理（防止旧探测回调污染新状态）
- checkpoint-not-caught-up guard（增量未追上时不转 ONLINE）

**`NetGuardEngine`**（纯逻辑核心，可单测）：
- `onProbeResult(success: Boolean, generation: Int): NetState` — 状态转移逻辑（2 次成功防抖在此）
- 单元测试覆盖：失败立即 OFFLINE、首次成功不转 ONLINE、第二次成功转 ONLINE、代际过期忽略

### 2.4 error（补全序列化与恢复元数据）

**已有**：`AppError.kt` sealed class（7 个 ErrorKind）。

**补全**：

```
commonMain/.../
├── AppError.kt              # 已有（补全方法）
└── error/
    ├── ErrorMetadata.kt     # 新增：恢复元数据
    └── ErrorSerializer.kt   # 新增：扁平序列化
```

**`ErrorMetadata`**（对标 src/error.rs 恢复信息）：
- `retryAfter: Duration?` — 服务端 Retry-After 头解析值
- `requestSemantics: RequestSemantics` — 请求语义（可重试/需刷新 token/不可恢复）
- `transportKind: DriveTransportKind?` — 传输错误子类（DNS/连接/超时/打断）

**`ErrorSerializer.toMap(error: AppError): Map<String, Any?>`** — 扁平序列化，供跨语言合同用（如 DELETE_TRACE_ERROR_PREFIX 契约）。字段：`kind`、`message`、`status?`、`retryAfterMs?`。

### 2.5 data（SQLDelight + CAS + migrations）

**方案**：SQLDelight（用户确认）。

```
commonMain/.../data/
├── DbSchema.kt              # 已有：DDL 字符串（保留，SQLDelight .sq 为准）
├── Models.kt                # 已有：data class
├── repository/
│   ├── SyncItemRepository.kt   # 接口：sync_items 增删改查 + CAS
│   ├── TransferRepository.kt   # 接口：transfer_queue + CAS + 状态迁移
│   ├── InodeMapRepository.kt   # 接口：local_inode_map（docs/11）
│   ├── FreeUpStagingRepository.kt  # 接口：free_up_staging
│   └── DatabaseMigrator.kt     # 接口：migrations v2→v6
└── .sq 文件（SQLDelight 查询）

macosMain/.../data/
├── DatabaseDriver.kt       # actual：SQLDelight native SQLite driver
└── repository/             # actual：仓库实现
```

**`.sq` 查询文件**（SQLDelight 约定，放在 `shared/src/commonMain/sqldelight/io/github/yuanbaobaao/petallink/` 源集下，包名对应目录）：

`sync_items.sq`：
- `selectAll`、`selectByFileId`、`selectByLocalPath`
- `insert`、`updateColumns`（动态拼 ColumnPatch）
- `casUpdateState`：`UPDATE sync_items SET sync_status=:status, state_revision=state_revision+1 WHERE id=:id AND state_revision=:expected`
- `deleteByFileId`

`transfer_queue.sq`：
- `insert`、`casTransitionState`（CAS 状态迁移）
- `updateRunningProgress`（刻意不递增 revision）
- `pruneHistory`（保留 100 条）

`local_inode_map.sq`：`lookup`、`upsert`、`purgeMissing`

`free_up_staging.sq`：`insert`、`selectByName`、`deleteByName`

**CAS 乐观锁**（docs/04 §6）：
- 所有状态变更走 `WHERE id=? AND state_revision=?`，受影响行数 == 0 即冲突（抛 `AppError.Data("CAS conflict")`）
- `ColumnPatch<T>` 三态映射：Keep → SQL 不含该字段；Set → `field=:value`；Clear → `field=NULL`

**migrations**（docs/04 §migrations + docs/11 §3.3）：
- 新库：直达 v6（执行 DbSchema.ALL_CREATE）
- 旧库 v2→v3→v4→v5→v6：每步 ALTER + CREATE
- v5 状态码归一化：0/1/2→Pending(0), 3→Completed(6), 4→Failed(7), 5→Canceled(8)
- v5→v6：增量新增 `local_inode_map` + `free_up_staging`（纯增量，不破坏数据）
- 不回填历史 inode（启动后首次扫描自动填充）

---

## 三、依赖与集成

### build.gradle.kts 变更

`shared/build.gradle.kts` 新增：
- SQLDelight 插件 + native driver 依赖
- 测试依赖：kotlin-test（commonTest 源集）

### 文件清单（本阶段新增/修改）

| 文件 | 类型 | 说明 |
|---|---|---|
| `commonMain/.../config/UserConfig.kt` | 新增 | 用户可配置项 |
| `commonMain/.../config/ConfigStore.kt` | 新增(expect) | 配置持久化接口 |
| `commonMain/.../config/ConfigValidator.kt` | 新增 | 校验规则 |
| `commonMain/.../core/logging/*` | 新增(4文件) | 日志接口+实现 |
| `commonMain/.../core/net_guard/*` | 新增(3文件) | 网络守卫 |
| `commonMain/.../error/ErrorMetadata.kt` | 新增 | 恢复元数据 |
| `commonMain/.../error/ErrorSerializer.kt` | 新增 | 扁平序列化 |
| `commonMain/.../data/repository/*.kt` | 新增(5接口) | 仓库接口 |
| `commonMain/.../data/*.sq` | 新增(4文件) | SQLDelight 查询 |
| `macosMain/.../config/ConfigStore.kt` | 新增(actual) | 配置持久化 |
| `macosMain/.../core/logging/*` | 新增(4文件) | 日志实现 |
| `macosMain/.../core/net_guard/*.kt` | 新增(actual) | TCP 探测 |
| `macosMain/.../data/DatabaseDriver.kt` | 新增(actual) | SQLDelight driver |
| `macosMain/.../data/repository/*.kt` | 新增(actual) | 仓库实现 |
| `commonTest/.../*Test.kt` | 新增 | 单元测试 |
| `shared/build.gradle.kts` | 修改 | SQLDelight + test 依赖 |

---

## 四、验证标准

1. **编译**：`./gradlew :shared:compileKotlinMacosArm64 :shared:compileKotlinMacosX64` 通过
2. **测试**：`./gradlew :shared:macosArm64Test` 通过，覆盖：
   - `ConfigValidatorTest`：各校验规则的通过/失败用例
   - `NetGuardEngineTest`：状态转移（失败立即 OFFLINE、2 次成功防抖、代际过期）
   - `CasTest`：CAS 冲突检测（旧 revision 更新失败）
   - `MigrationTest`：v5 状态码归一化映射正确
   - `InodeMapRepositoryTest`：upsert/lookup/purgeMissing
3. **无回归**：现有 commonMain/macosMain 代码仍编译通过

---

## 五、风险与对策

| 风险 | 对策 |
|---|---|
| SQLDelight native driver 在 macOS 的兼容性 | 先验证最小 driver 初始化，若不可用退回 cinterop sqlite3 |
| cinterop sqlite3 与 SQLDelight 共存冲突 | SQLDelight 自带 native driver，不需手动 cinterop |
| 日志文件路径权限（沙盒） | 路经 cache_paths 规则，写入 Application Support 目录 |
| CAS 测试需真实 DB | 用 SQLDelight 的 in-memory :memory: driver 跑测试 |

---

## 六、后续阶段衔接

阶段 1 完成后，阶段 2（华为 API）可直接使用：
- `AppError` + `ErrorSerializer`（API 错误处理）
- `NetGuard`（API 调用前检查在线状态）
- `Logger`（API 请求/响应日志，token 脱敏）
- `SyncItemRepository` / `TransferRepository`（API 结果落库）
- `UserConfig`（API 客户端配置：超时、回调端口）
