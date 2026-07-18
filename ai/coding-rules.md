# 编码规则

> 所有回复使用**中文**，代码注释使用**简短、准确的中文**，代码标识符和专有技术名词保留标准英文写法。
> 
> **UI 开发必须遵循 `docs/plan/09-设计系统.md` 与 `src/jvmMain/.../ui/theme/DesignTokens.kt` 中定义的设计令牌**（颜色、间距、圆角、字号统一取自 `DesignTokens`，禁止在 Composable 内硬编码带单位的数值）。

---

## 一、通用规则

### 1.1 语言与术语

- 变量、函数、类型、字段、文件名和模块名使用英文；引用现有标识符时必须保持源码原样，并使用反引号包裹。
- 专有技术名词不得生硬翻译。
- 协议字段和服务端标识保持官方大小写，例如 `fileId`、`parentFolder`、`serverId`、`uploadId`、`session_url`、`nextCursor`、`newStartCursor`。
- 禁止为了"中文化"而创造含义模糊的译名，例如把 `BFS` 写成"广度优先"、把 `PATCH` 写成"更新写入"、把 `token` 写成"令牌"。

### 1.2 可审阅性

- 代码首先服务于人工审阅：职责边界清楚，命名直接，控制流可顺序阅读，重要约束在靠近实现的位置说明。
- 单个 Kotlin 文件原则上不超过 **1000 行**；达到上限前应按单一职责拆分。禁止继续堆叠成超长综合文件。长生命周期聚合类（如 `ApplicationRoot`）如确实需要更长，应优先拆出子 ViewModel / 子协调器，而非在一个类里堆叠全部逻辑。
- 门面文件（如 `ApplicationRoot.kt`、`CommandService.kt`）只保留模块声明、公开导出、共享类型/常量和少量编排；具体网络请求、协议解析、持久化、恢复等职责放入子模块。
- 拆分文件不得顺便改变业务逻辑。函数体、SQL、HTTP 请求、锁和 `suspend`/`await` 顺序、错误映射、日志语义及可见性必须保持不变。
- 避免无意义抽象。只有当拆分能形成稳定职责边界、降低文件长度或让控制流更易审阅时才新增模块。

### 1.3 格式化与缩进

- 全仓统一 **4 空格缩进**，禁止 Tab。与 `gradle.properties` 的 `kotlin.code.style=official` 保持一致。
- 行尾不留空格，文件以单一换行符结尾。
- import 顺序：先 Kotlin/Java 标准库，再第三方，最后本项目；按字母序排列，禁止通配 `import x.*`（IDE 自动管理即可，不要手写通配）。
- 遵循 `ktlint`/官方 Kotlin 编码风格的默认约定（如 `{` 不换行、`when` 分支不强制换行、单表达式函数体写在一行）。

---

## 二、Kotlin 编码规范

### 2.1 文件与声明注释（强制）

- **文件/类/接口/object/enum/数据类/顶层函数**等命名声明必须有 KDoc 注释；`expect` 声明同样必须有 KDoc（`actual` 可不重复，见 2.2）。
- KDoc 一律写成多行格式，**`/**` 后必须换行**，每行以 ` * ` 前缀；**禁止单行 `/** 中文 */`**。

```kotlin
/**
 * 网络守卫接口（expect，jvmMain 提供 actual）。
 *
 * 职责：周期性 TCP 探测华为域名 443，通过 [NetGuardEngine] 做防抖判定，
 * 暴露当前 [NetState]。
 */
expect class NetGuard(...) { ... }
```

- 文件级说明（对标来源、整体职责）写在文件顶部第一个声明之前的 KDoc 中，**不要**使用 Rust 风格的 `//!`（全仓不使用该语法）。

```kotlin
package io.github.yuanbaobaoo.petallink.data

/**
 * 数据模型（对标原项目 src/data/ 与 src/sync/state.rs）
 *
 * 详见 docs/04-数据模型与持久化.md。
 */
data class SyncItem(...)
```

### 2.2 注释规范（强制）

**一句话原则：文档写在"对外定义"上，实现不重复写。**

> "对外定义"指会被别的代码或别的人调用的地方——`expect` 声明、接口方法、抽象方法、公开 `class` 的公开成员。这些是别人理解你代码的入口，必须写清楚。反过来，`actual` 实现、`override` 的方法、内部 helper，入口处已经说明过了，不要再抄一遍。

- **对外定义必须有 KDoc**（含 `@param`/`@return`）；**实现处不重复**，只有当实现的行为比定义多了一些约束或坑时，才用 `//` 在方法体内补充。
- 私有 helper、简单赋值、显而易见的分支 → 不机械补注释；复杂协议分支、状态机转换、锁边界、重试/恢复逻辑必须用 `//` 说明"为什么"。
- 注释重点回答"为什么存在、保证什么、失败时怎样、有什么副作用"，不要复述函数名或逐行翻译代码。
- 复杂协议背景用文件级 KDoc 集中讲一次，避免在每个方法里重复。
- 踩坑/对标备忘直接写进对应声明的 KDoc（如 `踩坑：parentFolder 用 queryParam 语法`）。
- 注释必须与代码同步。移动或拆分实现时，同步更新路径、职责和术语，禁止保留失效说明。

**字段注释 + 空行规则**（沿用 Java 既有习惯）：

- 用 `/** */` 注释的字段之间**必须空一行**；用 `//` 注释的字段之间**不空行**。
- 常量、枚举值、数据类字段含义不直观时加注释；显而易见的私有数据容器字段不强求。
- 局部变量一般不加注释；承载关键不变量（如代际 `gen`、乱序保护 `revision`）时用 `//` 说明用途。

**枚举注释（强制）**：枚举类本身必须有块级 KDoc，说明这个枚举是干什么的、按什么维度划分；**每个枚举值也必须用 `/** */` 块级注释**，`/**` 后换行；**禁止**用 `//` 行注释标注枚举值：

```kotlin
/**
 * 应用错误分类（按错误来源与处理方式划分）。
 */
enum class ErrorKind {
    /**
     * 网络层（DNS/连接/超时/打断）→ 可重试
     */
    NETWORK,

    /**
     * 401 / token 失效 → 触发刷新或要求重新登录
     */
    AUTH,

    /**
     * 远端业务错误（非 2xx，含配额、文件不存在等）
     */
    REMOTE,

    /**
     * 其他未分类（不应出现，出现即视为 bug）
     */
    INTERNAL,
}
```

### 2.3 命名规范

| 类别 | 约定 | 示例 |
|------|------|------|
| 类 / 接口 / 注解 | PascalCase | `NetGuardEngine`、`DriveClient`、`CommandService` |
| 自研 Compose 组件 | PascalCase + `Mate` 前缀 | `MateButton`、`MateDialogHost` |
| 函数 | camelCase | `fetchTableList`、`applyState` |
| 常量（`const val` / 顶层 `val`） | UPPER_SNAKE_CASE | `BRAND`、`SPACING_XS`、`PROBE_HOST` |
| 可变私有状态（`MutableStateFlow` 等） | `_camelCase` 下划线前缀 | `_isLoggedIn`、`_state`、`_tasks` |
| 不可变私有字段 | camelCase（无前缀） | `scope`、`httpClient`、`engine` |
| 数据类字段 / 协议字段 | camelCase，协议字段保留官方大小写 | `fileId`、`parentFolderId`、`nextCursor` |
| 包名 | 全小写，不缩写 | `io.github.yuanbaobaoo.petallink.sync.engine` |

**函数命名动词前缀**（沿用既有风格）：

| 前缀 | 用途 | 示例 |
|------|------|------|
| `fetch` | 网络请求获取数据 | `fetchFileList()` |
| `load` | 加载数据（可能含缓存/DB） | `loadFailedItems()` |
| `apply` | 把快照/状态应用到 UI | `applySnapshot()`、`applyState()` |
| `handle` | 事件处理 | `handleSubmit()`、`handleFileChange()` |
| `on` | 回调入口 | `onProbeResult()`、`onMenuSelect()` |
| `do` | 执行操作 | `doLogout()`、`doDelete()` |
| `open` / `close` | 打开/关闭弹窗或页面 | `openEditRow()`、`closeModal()` |
| `refresh` | 刷新当前视图 | `refresh()` |

### 2.4 常量与单例

- 模块级/类级常量使用 `UPPER_SNAKE_CASE`，集中放在 `companion object` 内用 `const val` 声明；禁止魔法字符串/数字硬编码。
- 真正的全局无状态单例（常量容器、纯配置）用 `object`（如 `DesignTokens`、`DriveClientConfig`）。
- 应用级长生命周期对象（含状态、需关闭）**不用 `object`**，由 Composition Root（`ApplicationRoot`）持有实例字段，并实现 `AutoCloseable` 做逆序关闭。
- 工厂方法放 `companion object`（如 `production()` / `development()` / `fromEnvironment()`、`create(...)`）。
- 单例若需支持测试覆盖，提供 `configureForTest(...)` 入口，避免暴露可变全局字段。

### 2.5 函数声明风格

- 普通函数用 `fun name(...) { ... }`；短函数用表达式体 `fun x() = ...`。
- 耗时/IO 操作用 `suspend fun`；UI 回调、集合变换、`launch`/`forEach`/`map` 内部一律用 lambda（不在公共 API 暴露不必要的 `suspend`）。
- 默认参数优先于函数重载；命名参数调用提升可读性（如 `refreshInternal(triggerSync = true)`）。
- 需要回调值的场景优先用函数类型（如 `onResult: (Boolean) -> Unit`）而非定义一次性接口。

### 2.6 协程与 Flow（强制）

- **状态对外只暴露 `StateFlow`**：私有 `MutableStateFlow` + 公开 `asStateFlow()`，禁止把 `MutableStateFlow` 直接暴露给其他模块。
- **事件流用 `SharedFlow`**（如 `uploadFailures`）；事件需去重/乱序保护时，在产生侧用递增 `revision`/`requestId` 丢弃过期项。
- 作用域：应用级用 `CoroutineScope(SupervisorJob() + Dispatchers.Default)`，子操作在其内 `launch`；长生命周期对象在 `close()` 中取消 scope。
- 互斥优先用 `Mutex.withLock { }`，不要用 `synchronized` 阻塞协程。
- 周期任务用 `while (isActive) { ...; delay(...) }`，不要裸 `Thread.sleep`。
- 简单容错取值用 `runCatching { }`，但**写操作、状态机变更不得吞异常**（见 2.7）。
- JVM 跨线程共享的可变状态用 `AtomicReference`/`AtomicInteger`/`AtomicBoolean`。

### 2.7 错误处理：`AppError` + `AppResult` 双层（强制）

本项目沿用既有错误模型，思路对标 xe-cloud "一种业务异常 + 统一结果体"，但用 Kotlin 密封类实现：

- **底层 service 直接 `throw AppError.Xxx(...)`**（按 `ErrorKind` 分类：`NETWORK`/`AUTH`/`REMOTE`/`CONFLICT`/`DATA`/`LOCAL_IO`/`CANCELED`/`INTERNAL`）。
- **命令/编排层用 `AppResult<T>`（`Ok` / `Err(AppError)`）回传**，通过 `safe` / `drive` / `dbSafe` 等 helper 统一捕获并分类：

```kotlin
private suspend fun <T> drive(block: suspend () -> T): AppResult<T> = try {
    AppResult.Ok(block())
} catch (e: AppError) {
    AppResult.Err(e)
} catch (e: Throwable) {
    AppResult.Err(AppError.Remote(e.message ?: "远端未知错误", cause = e))
}
```

- 前置条件校验用 `require(...)` 并给出中文失败信息（如 `require(pageSize in 1..100)`、路径越界 `require(it.startsWith(root)) { "文件路径越界" }`）。
- **禁止**：吞掉异常只返回默认值（除非是 UI 展示用的退化值，且必须留日志）；用 `Throwable.message` 直接当作用户文案（文案在 UI 层按 `ErrorKind` 本地化）。

### 2.8 可见性

- 默认 `public`（不显式写）；需要对外暴露的类型/函数直接省略修饰符。
- 构造参数、内部状态、helper 一律 `private`；跨 source set 但非公开的工厂方法用 `internal`（便于确定性测试）。
- `companion object` 的常量可按可见性分组：公开常量默认 public，私有伴生用 `private companion object`。
- 拆分模块时使用满足实现所需的最小可见性，避免因拆分扩大公共接口。

### 2.9 数据类、密封类、枚举

- 数据模型首选 `data class`；不可变值用 `val`，需修改时用 `.copy(...)`，避免可变 `var`。
- 错误分层、结果类型、有限状态机用 `sealed class`/`sealed interface`（如 `AppError`、`AppResult`、`SetupPhase`）。
- 分类、状态用 `enum class`；带额外数据的多形态状态用 `sealed class`。
- **枚举类必须有块级 KDoc，每个枚举值也必须有块级 KDoc**（见 2.2 枚举示例），不得用 `//` 行注释标注枚举值。
- 集合返回值优先返回只读类型（`List`/`Map`/`Set`），内部可变集合不外泄。

### 2.10 `expect` / `actual`（KMP 专属）

- 平台差异声明放 `commonMain` 用 `expect`，实现放 `jvmMain` 用 `actual`；目前**只存在 JVM 源集**，暂无 `macosMain`/`iosMain`。
- `expect` 声明必须有 KDoc 说明它的用途与约定；`actual` 不重复文档。
- `expect` 与 `actual` 的包名必须一致；文件名不强求一致（既有 `Database.kt` ↔ `PetalLinkDb.kt` 的先例允许保留，但新建文件建议同名以便审阅）。
- 不要为"以后可能多平台"而过度抽象；当前只在 macOS/JVM 有差异处才用 `expect`。

---

## 三、Compose UI 规范

### 3.1 状态与 ViewModel

- 采用单一顶层 `DesktopUiState` + 顶层 `DesktopAppViewModel`（`collectAsState()` 消费）；各子 ViewModel（`SyncViewModel`/`TransferViewModel`/`FileBrowserViewModel`）聚合到顶层 state。
- UI 状态不可变：用 `data class` + `.copy()` 更新，禁止在 Composable 内持有可变业务状态。
- 副作用在 `LaunchedEffect`/`DisposableEffect`/`rememberCoroutineScope` 中执行，不要在组合函数体内直接启动协程或做 IO。

### 3.2 组件组织

- 自研组件统一放 `ui/components/mate/`，命名带 `Mate` 前缀，保持 API 稳定。
- 页面级 Composable 放 `ui/pages/main/`，按屏命名（`LoginScreen`、`MainScreen`、`FileListScreen`、`SettingsScreen`、`LogViewerScreen`）。
- Composable 函数以大写开头、PascalCase；预览函数以 `Preview` 结尾并加 `@Preview`。

### 3.3 设计令牌

- 颜色、间距、圆角、字号统一取自 `DesignTokens`（`ui/theme/DesignTokens.kt`），禁止在 Composable 内硬编码 `Color(0xFF0053DB)`、`12.dp` 等字面量。
- 主题切换走 `ui/theme/Theme.kt`，不要在业务组件里直接访问 `MaterialTheme.colors` 之外的硬编码颜色。
- 区块分隔沿用既有 ASCII 框线注释风格（`// ------ 标题 ------`）保持 `DesignTokens` 的可读性。

### 3.4 Compose API 约定

- 事件回调参数命名：`onClick`、`onChange`、`onSelect`，类型为 `() -> Unit` 或 `(T) -> Unit`。
- `Modifier` 作为第一个可选参数（`modifier: Modifier = Modifier`），便于调用方链式定制。
- 列表用 `LazyColumn`/`LazyRow`，`key` 指定稳定标识（`fileId`/`id`），避免用 index 作 key。

---

## 四、Room KMP / 持久化规范

### 4.1 common-first 组织

- `@Entity`、`@Dao`、`RoomDatabase`、类型转换器和 Repository 实现统一放在 `commonMain`，上层业务只能依赖 common 类型。
- 平台 source set 只允许提供 `RoomDatabase.Builder`、数据库路径和平台时钟等系统能力；禁止复制 Entity、DTO、DAO 或 Repository 业务逻辑。
- 实体必须使用单列主键；业务唯一约束用唯一索引表达，禁止复合主键。
- schema 导出到根目录 `schemas/` 并纳入版本控制。当前不兼容旧 SQLDelight 数据库，不维护旧库迁移链。

### 4.2 SQL 编写约定

- SQL 类型映射：Kotlin `Long`/时间戳 → `INTEGER`（8 字节），`String` → `TEXT`，`Boolean` → `INTEGER`（0/1），浮点 → `REAL`。
- **所有时间字段统一存毫秒时间戳（`Long` / `INTEGER`）**，禁止存字符串时间；命名 `createTime`/`updateTime`/`lastSyncTime`（SQL 列对应 `create_time` 等下划线命名）。
- 查询 SQL 统一写在 DAO 的 `@Query` 中；复杂 SQL 应拆分为语义明确的 DAO 方法，禁止在 Repository 或业务层拼接裸 SQL。
- 多步写操作使用 `@Transaction`，CAS 更新必须同时检查主键和 revision/源快照，并以受影响行数判断是否成功。
- schema 演进通过 Room `Migration` 和导出的 schema 管理；任何版本升级必须补迁移测试，禁止使用 destructive migration 掩盖缺失迁移。

### 4.3 Repository 封装（强制，借鉴 xe-cloud 三层思想）

本项目采用两层（`Repository` + Room `Dao`），核心边界如下：

- 对外只暴露 `Repository`（`data/repository/`），内部持有 Room `Dao`。
- **外部模块禁止直接访问 `Dao`/`RoomDatabase`**，必须通过 `Repository`；即便是同一模块的其他 Repository，也不得互相调用对方的 DAO。
- Room Entity 可直接复用现有 common 业务数据类，禁止为 JVM/Native 各建一套同义类型。
- 查询结果交给上层时不得外泄 Room 内部类型、Cursor 或数据库连接。

---

## 五、网络（Ktor）规范

### 5.1 API 客户端组织

- 每个 API 域一个客户端类（如 `FilesApi`、`ChangesApi`、`UploadApi`），放 `drive/` 包，方法为 `suspend fun`。
- 公共配置（baseUrl、超时、拦截器）集中在 `DriveClient` / `DriveClientConfig`；禁止每个 API 类各自创建 `HttpClient`。
- 错误统一抛 `AppError`：HTTP 层错误 → `AppError.Network(...)`，非 2xx 业务错误 → `AppError.Remote(...)`，401/token 失效 → `AppError.Auth(...)`。

### 5.2 请求约定

- 序列化用 `kotlinx.serialization`；请求/响应模型为 `@Serializable data class`，字段保留服务端官方命名。
- 协议特殊语法必须在 KDoc 标注（如 `踩坑：parentFolder 用 queryParam 语法（'id' in parentFolder）`）。
- 分页参数显式校验：`require(pageSize in 1..100)`；游标字段保持官方命名 `nextCursor`/`newStartCursor`。
- **大整数 ID/时间戳序列化**：本项目用 `kotlinx.serialization` 默认行为，`Long` 字段在 JSON 中为数字；若未来对接 JS/精度敏感消费方，再按需加 `@Serializable(with = ...)` 统一转字符串（参考 xe-cloud 全局 `WriteLongAsString` 思路，不在每个字段散落注解）。

---

## 六、测试规范

### 6.1 测试组织

- `commonTest/`：跨平台纯逻辑测试（不依赖 JVM 平台特性），包结构镜像 `commonMain`。
- `jvmTest/`：依赖 JVM/macOS 平台的测试（文件系统、JNA、Ktor 引擎、ApplicationRoot 装配）。
- 测试文件命名 `XxxTest.kt`，与被测类同包；兼容性测试放 `compat/` 子包（如 `LegacyTauriCompatibilityTest`）。

### 6.2 测试编写约定

- 断言库统一用 **`kotlin.test`**（`assertEquals`/`assertTrue`/`assertFalse`），`@Test` 来自 `kotlin.test`；不引入 JUnit 4 注解或 Kotest。
- **测试方法名允许用中文**（本项目既有风格），直接用裸中文标识符或反引号包裹，描述被验证的合同：

```kotlin
@Test
fun 失败立即转OFFLINE() { ... }

@Test
fun 跨日期写不同文件并清理30天以前日志() { ... }
```

- 测试类加 KDoc 说明对标对象（如 `/** NetGuardEngine 纯逻辑单测（对标 docs/06 §网络守卫） */`）。
- HTTP 测试用 `ktor-client-mock`，不访问真实网络。
- 临时目录用 `Files.createTempDirectory("petallink-xxx-test-")`，**不手写递归删除**（见第八章铁律）；依赖系统临时目录清理或显式删除文件即可。

### 6.3 测试保留原则

- 只保留核心业务合同、协议边界、状态机、恢复语义和高风险回归测试；删除重复、低价值或只验证实现细节的测试。
- 真实云端、真实账号或人工环境测试必须用 `@Ignore` 标注，并通过明确的环境变量显式启用；默认测试不得访问真实外部服务或产生外部副作用。
- 不硬编码测试数量；文档统一以 `./gradlew :shared:jvmTest -- --list-tests`（或实际命令输出）为准。

### 6.4 验证命令

常规验证至少包括：

```bash
./gradlew :shared:jvmTest                    # 单元 + 集成测试
./gradlew :shared:compileKotlinJvm           # 编译检查
```

涉及行为变更时再运行相关测试；发布前的完整矩阵见 `ai/release-rule.md`。

---

## 八、文件系统与打包安全铁律

> **教训来源：2026-07-17 事故。** `repackDmgForEntitlements` 任务（已删除）在 staging 目录创建了 `Applications -> /Applications` 符号链接，随后用 `staging.deleteRecursively()` 清理。`deleteRecursively` 递归遍历时若跟随符号链接，会进入真实的 `/Applications` 递归删除，导致用户已安装的第三方 app 全部丢失。此类"符号链接 + 递归删除"的组合是致命隐患，必须从规则上根除。

### 8.1 agent 行为边界（强制）

- **agent 只允许"打包"，不允许"执行自己写的代码"。**"打包"指调用既有的、用户已知的构建命令（如 `./gradlew :shared:packageDmg`）。agent 不得运行任何由自己新编写、尚未经用户逐行审查确认的任务、脚本或可执行产物。
- 新增任何构建任务、脚本、会删除文件的自定义逻辑，必须先在对话中以完整源码形式交用户审查，得到明确同意后才能执行；不得先写进文件再顺带跑起来。

### 8.2 禁止的文件系统操作（强制）

- **禁止在临时目录创建指向系统目录的符号链接后再递归删除该临时目录。** 受保护系统目录包括但不限于：`/Applications`、`/System`、`/Library`、`/usr`、`/bin`、`/Users`、`~`、用户数据目录。
- **禁止对包含符号链接的目录使用 `deleteRecursively`、`rm -rf`、`find -delete` 等递归删除。** 递归删除前必须确保目录树内不存在符号链接，或对符号链接显式跳过。
- **禁止用 `deleteRecursively` 等递归删除清理 staging / build 产物时，该目录内曾创建过任何指向目录树之外的符号链接。**

### 8.3 删除类任务的强制约束

凡涉及文件系统删除的自定义构建任务或脚本，必须同时满足：

1. **删除前显式校验目标路径位于允许的根目录内**（仅限项目 `build/`、系统临时目录如 `/tmp/`、或用户显式指定的目录），校验失败立即报错中止，不得继续。
2. **删除时用允许的前缀白名单二次校验**，例如 Kotlin 中 `require(target.startsWith(allowedRoot))`，bash 中 `case "$target" in "$ALLOWED_ROOT"/*) ...`。
3. **递归删除前先扫描目录内是否存在符号链接，发现则报错中止或显式跳过符号链接**，绝不允许带着符号链接直接递归删除。
4. **临时目录命名必须唯一且路径可控**（如 `build/.../tmp/<task>-<timestamp>`），不得复用固定路径或越界到项目外。

### 8.4 Shell 安全规范

- 所有 bash 脚本头部必须 `set -euo pipefail`，参数化、路径限定，禁止 `cd` 到绝对系统目录后执行删除。
- 临时目录用 `mktemp -d`，清理用 `trap cleanup EXIT` 且仅 `kill` 进程或删除自己创建的文件，不递归删除系统目录。

### 8.5 自查清单（提交前必检）

任何对 `build.gradle.kts`、`settings.gradle.kts`、Shell 命令或自定义 Gradle 任务的修改，提交前必须逐条对照：

- [ ] 是否引入了指向项目目录之外（尤其系统目录）的符号链接？如有 → 禁止。
- [ ] 是否存在对含符号链接目录的递归删除？如有 → 禁止。
- [ ] 删除类任务是否在删除前做了路径白名单校验？无 → 必须补。
- [ ] 新写的任务/脚本是否未经用户审查就直接执行了？如是 → 违规，立即停。
- [ ] 新脚本是否带 `set -euo pipefail`、临时目录是否唯一可控？

违反本章任意一条，视为严重事故。

---

## 九、Git 提交规范

**禁止自动提交代码。** 所有代码改动完成后，由用户明确指示（如"提交代码"、"commit"）后才能执行 `git commit`，没有指示则保持工作区状态。

- 不主动切换分支，除非用户明确要求。
- 改动完成后，告知用户改动文件清单和验证情况，由用户决定是否提交。
- 不主动执行 `git add` / `git commit` / `git push`，除非用户明确要求。
- 用户只说"提交"时仅提交，不附带 push；需要 push 时由用户另行指示。
- 用户要求 commit 时，commit message 必须以中文为主要语言编写；如需标号或引用 issue，置于行首（如 `# 修复…` / `重构…`），正文说明动机和影响范围。
- **`docs/superpowers/` 目录永不纳入版本控制**。禁止用 `git add -f` 强制添加该目录下的任何文件；该目录仅作本地工作文件，不入库。

---

## 十、修改代码约束

1. 优先"最小改动"，不影响已有功能，不修改无关文件。
2. 修改前必须说明影响范围（涉及哪些模块、对外接口是否变化）。
3. 拆分文件不得夹带业务修复或行为优化；行为变更单独成提交。
4. 使用已有工具类/公共库，禁止重复造轮子（如时间格式化、路径校验、日志脱敏等优先复用 `core/` 内既有实现）。
5. 生成代码必须遵守本规范，符合当前模块架构，保持风格一致。
