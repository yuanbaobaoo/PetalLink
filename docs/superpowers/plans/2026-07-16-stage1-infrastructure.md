# 阶段 1 实施计划：基础设施层

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 补全 config 校验 / logging 三层 / net_guard / error 序列化 / SQLDelight data 层，为阶段 2-6 提供基础设施。

**Architecture:** commonMain 承载纯逻辑（接口 + 校验 + 状态机 + 纯算法），macosMain 承载平台实现（SQLDelight native driver / os_log / TCP 探测 / 文件持久化）。SQLDelight 负责四表持久化与 CAS。

**Tech Stack:** Kotlin 2.1.21 / KMP / SQLDelight 2.0.x / kotlinx-coroutines / kotlinx-serialization

---

## 文件结构

```
shared/
├── build.gradle.kts                                    # 修改：+SQLDelight 插件/依赖 + test
├── src/
│   ├── commonMain/
│   │   ├── kotlin/.../petallink/
│   │   │   ├── AppError.kt                             # 修改：+statusAccessor
│   │   │   ├── config/
│   │   │   │   ├── AppConfig.kt                        # 已有
│   │   │   │   ├── UserConfig.kt                       # 新增
│   │   │   │   ├── ConfigStore.kt                      # 新增(expect)
│   │   │   │   └── ConfigValidator.kt                  # 新增
│   │   │   ├── error/
│   │   │   │   ├── ErrorMetadata.kt                    # 新增
│   │   │   │   └── ErrorSerializer.kt                  # 新增
│   │   │   ├── core/logging/
│   │   │   │   ├── LogLevel.kt                         # 新增
│   │   │   │   ├── LogRecord.kt                        # 新增
│   │   │   │   ├── Logger.kt                           # 新增
│   │   │   │   └── LogAppender.kt                      # 新增
│   │   │   ├── core/net_guard/
│   │   │   │   ├── NetState.kt                         # 新增
│   │   │   │   ├── NetGuard.kt                         # 新增(expect)
│   │   │   │   └── NetGuardEngine.kt                   # 新增(纯逻辑)
│   │   │   └── data/
│   │   │       ├── DbSchema.kt                         # 已有
│   │   │       ├── Models.kt                           # 已有
│   │   │       ├── Database.kt                         # 新增(expect: 仓库工厂)
│   │   │       └── repository/
│   │   │           ├── SyncItemRepository.kt           # 新增(接口)
│   │   │           ├── TransferRepository.kt           # 新增(接口)
│   │   │           ├── InodeMapRepository.kt           # 新增(接口)
│   │   │           └── FreeUpStagingRepository.kt      # 新增(接口)
│   │   └── sqldelight/.../petallink/
│   │       ├── sync_items.sq                           # 新增
│   │       ├── transfer_queue.sq                       # 新增
│   │       ├── local_inode_map.sq                      # 新增
│   │       └── free_up_staging.sq                      # 新增
│   ├── macosMain/kotlin/.../petallink/
│   │   ├── config/ConfigStore.kt                       # 新增(actual)
│   │   ├── core/logging/LoggerImpl.kt                  # 新增(actual)
│   │   ├── core/net_guard/NetGuardImpl.kt              # 新增(actual)
│   │   └── data/
│   │       ├── DatabaseDriver.kt                       # 新增(actual)
│   │       └── repository/*RepositoryImpl.kt           # 新增(actual, 4文件)
│   └── commonTest/kotlin/.../petallink/
│       ├── config/ConfigValidatorTest.kt
│       ├── net_guard/NetGuardEngineTest.kt
│       ├── error/ErrorSerializerTest.kt
│       └── data/InodeMapLogicTest.kt
```

---

## Task 1: SQLDelight 接入与最小编译验证

**Files:**
- Modify: `gradle/libs.versions.toml`
- Modify: `shared/build.gradle.kts`

- [ ] **Step 1: 版本目录加 SQLDelight + kotlin-test**

修改 `gradle/libs.versions.toml`，在 `[versions]` 加 `sqldelight = "2.0.2"`，在 `[libraries]` 加：
```toml
sqldelight-native = { group = "app.cash.sqldelight", name = "native-driver", version.ref = "sqldelight" }
sqldelight-coroutines = { group = "app.cash.sqldelight", name = "coroutines-extensions", version.ref = "sqldelight" }
```
在 `[plugins]` 加：
```toml
sqldelight = { id = "app.cash.sqldelight", version.ref = "sqldelight" }
```

- [ ] **Step 2: 根 build.gradle.kts 加 SQLDelight 插件 apply false**

`build.gradle.kts` 的 plugins 块加一行：
```kotlin
alias(libs.plugins.sqldelight) apply false
```

- [ ] **Step 3: shared/build.gradle.kts 应用插件 + 配置 + 测试依赖**

plugins 块加 `alias(libs.plugins.sqldelight)`。
kotlin 块的 `sourceSets` 中 `commonMain` dependencies 末尾加：
```kotlin
implementation(libs.sqldelight.coroutines)
```
`macosMain`（通过 hierarchy）加 native driver——在 `macosMain` sourceSet 不需显式，改在 sourceSets 同级加 `commonTest`：
```kotlin
commonTest {
    dependencies {
        implementation(kotlin("test"))
        implementation(libs.kotlin.coroutines)
    }
}
```
kotlin 块末尾加 sqldelight 配置：
```kotlin
sqldelight {
    databases {
        create("PetalLinkDatabase") {
            packageName.set("io.github.yuanbaobaao.petallink.data")
        }
    }
}
```

- [ ] **Step 4: 加一个空 .sq 文件占位，验证编译**

创建 `shared/src/commonMain/sqldelight/io/github/yuanbaobaao/petallink/data/sync_items.sq`：
```sql
-- sync_items 表查询（对标 docs/04 §3）
selectAll:
SELECT * FROM sync_items;
```

- [ ] **Step 5: 编译验证**

Run: `./gradlew :shared:compileKotlinMacosArm64 2>&1 | grep -E "BUILD|FAILED|error"`
Expected: BUILD SUCCESSFUL

---

## Task 2: 四张表的 .sq 查询文件

**Files:**
- Create: `shared/src/commonMain/sqldelight/io/github/yuanbaobaao/petallink/data/sync_items.sq`（覆盖 Task1 占位）
- Create: `shared/src/commonMain/sqldelight/io/github/yuanbaobaao/petallink/data/transfer_queue.sq`
- Create: `shared/src/commonMain/sqldelight/io/github/yuanbaobaao/petallink/data/local_inode_map.sq`
- Create: `shared/src/commonMain/sqldelight/io/github/yuanbaobaao/petallink/data/free_up_staging.sq`
- Create: `shared/src/commonMain/sqldelight/io/github/yuanbaobaao/petallink/data/schema.sq`（CREATE TABLE 语句）

- [ ] **Step 1: schema.sq 放全部建表 + 索引**

创建 `schema.sq`，内容为 DbSchema.ALL_CREATE 的纯 SQL（去掉 Kotlin 字符串转义），含 CREATE TABLE IF NOT EXISTS sync_items / transfer_queue / sync_cursor / local_inode_map / free_up_staging 及全部索引。

- [ ] **Step 2: sync_items.sq 查询**

```sql
insertRow:
INSERT INTO sync_items(file_id, local_path, parent_file_id, is_folder, size, mtime, etag, sync_status, state_revision)
VALUES (:fileId, :localPath, :parentFileId, :isFolder, :size, :mtime, :etag, :syncStatus, :stateRevision);

selectByFileId:
SELECT * FROM sync_items WHERE file_id = :fileId LIMIT 1;

selectByLocalPath:
SELECT * FROM sync_items WHERE local_path = :localPath LIMIT 1;

casUpdateStatus:
UPDATE sync_items
SET sync_status = :newStatus, state_revision = state_revision + 1, last_error = :errorMsg
WHERE id = :id AND state_revision = :expectedRevision;

updateEtag:
UPDATE sync_items SET etag = :etag, state_revision = state_revision + 1
WHERE id = :id AND state_revision = :expectedRevision;

deleteByFileId:
DELETE FROM sync_items WHERE file_id = :fileId;
```

- [ ] **Step 3: transfer_queue.sq 查询**

```sql
insertRow:
INSERT INTO transfer_queue(file_id, local_path, direction, state, state_revision, attempt, bytes_total, bytes_done, error_message, upload_session_url, created_at, updated_at)
VALUES (:fileId, :localPath, :direction, :state, :stateRevision, :attempt, :bytesTotal, :bytesDone, :errorMessage, :uploadSessionUrl, :createdAt, :updatedAt);

casTransitionState:
UPDATE transfer_queue
SET state = :newState, state_revision = state_revision + 1, attempt = :attempt, error_message = :errorMsg, updated_at = :updatedAt
WHERE id = :id AND state_revision = :expectedRevision;

updateRunningProgress:
UPDATE transfer_queue
SET bytes_done = :bytesDone, updated_at = :updatedAt
WHERE id = :id;

selectActiveByState:
SELECT * FROM transfer_queue WHERE state = :state;

pruneHistory:
DELETE FROM transfer_queue
WHERE id NOT IN (
  SELECT id FROM transfer_queue ORDER BY updated_at DESC LIMIT :keepCount
);
```

- [ ] **Step 4: local_inode_map.sq 查询**

```sql
upsert:
INSERT INTO local_inode_map(inode, relative_path, file_id, scanned_at)
VALUES (:inode, :relativePath, :fileId, :scannedAt)
ON CONFLICT(inode) DO UPDATE SET relative_path=:relativePath, file_id=:fileId, scanned_at=:scannedAt;

lookupByInode:
SELECT * FROM local_inode_map WHERE inode = :inode LIMIT 1;

deleteByInode:
DELETE FROM local_inode_map WHERE inode = :inode;
```

- [ ] **Step 5: free_up_staging.sq 查询**

```sql
insertRow:
INSERT OR REPLACE INTO free_up_staging(staging_name, relative_path, file_id, source_mtime, source_size, created_at)
VALUES (:stagingName, :relativePath, :fileId, :sourceMtime, :sourceSize, :createdAt);

selectByName:
SELECT * FROM free_up_staging WHERE staging_name = :stagingName LIMIT 1;

deleteByName:
DELETE FROM free_up_staging WHERE staging_name = :stagingName;
```

- [ ] **Step 6: 编译验证 SQLDelight 生成代码**

Run: `./gradlew :shared:compileKotlinMacosArm64 2>&1 | grep -E "BUILD|FAILED|error"`
Expected: BUILD SUCCESSFUL

---

## Task 3: Repository 接口（commonMain）

**Files:**
- Create: `shared/src/commonMain/kotlin/.../data/repository/SyncItemRepository.kt`
- Create: `shared/src/commonMain/kotlin/.../data/repository/TransferRepository.kt`
- Create: `shared/src/commonMain/kotlin/.../data/repository/InodeMapRepository.kt`
- Create: `shared/src/commonMain/kotlin/.../data/repository/FreeUpStagingRepository.kt`
- Create: `shared/src/commonMain/kotlin/.../data/Database.kt`

- [ ] **Step 1: InodeMapRepository 接口**

```kotlin
package io.github.yuanbaobaao.petallink.data.repository

import io.github.yuanbaobaao.petallink.sync.identity.InodeRecord

interface InodeMapRepository {
    suspend fun lookup(inode: ULong): InodeRecord?
    suspend fun upsert(inode: ULong, relativePath: String, fileId: String)
    suspend fun delete(inode: ULong)
}
```

- [ ] **Step 2: FreeUpStagingRepository 接口**

定义 `FreeUpStagingRecord` data class（stagingName, relativePath, fileId, sourceMtime, sourceSize, createdAt）+ 接口（insert / findByName / deleteByName）。

- [ ] **Step 3: SyncItemRepository 接口**

方法：`insert`、`findByFileId`、`findByLocalPath`、`casUpdateStatus(id, expectedRevision, newStatus, errorMsg): Boolean`（返回 false 即 CAS 冲突）、`deleteByFileId`。

- [ ] **Step 4: TransferRepository 接口**

方法：`insert`、`casTransitionState(id, expectedRevision, newState, attempt, errorMsg): Boolean`、`updateRunningProgress(id, bytesDone)`（不递增 revision）、`selectActiveByState`、`pruneHistory(keepCount)`。

- [ ] **Step 5: Database expect 类（仓库工厂）**

```kotlin
package io.github.yuanbaobaao.petallink.data

expect class PetalLinkDatabase {
    // SQLDelight 生成的 PetalLinkDatabase 包装
}
expect class PetalLinkDb {
    val syncItems: ...SyncItemRepository
    val transfers: ...TransferRepository
    val inodeMap: ...InodeMapRepository
    val freeUpStaging: ...FreeUpStagingRepository
}
```
（具体实现细节在 macosMain actual，此处先声明接口聚合）

- [ ] **Step 6: 编译验证**

Run: `./gradlew :shared:compileKotlinMacosArm64 2>&1 | grep -E "BUILD|FAILED|error"`
Expected: BUILD SUCCESSFUL

---

## Task 4: net_guard 纯逻辑 + 单测（TDD）

**Files:**
- Create: `shared/src/commonMain/kotlin/.../core/net_guard/NetState.kt`
- Create: `shared/src/commonMain/kotlin/.../core/net_guard/NetGuardEngine.kt`
- Create: `shared/src/commonMain/kotlin/.../core/net_guard/NetGuard.kt`
- Test: `shared/src/commonTest/kotlin/.../core/net_guard/NetGuardEngineTest.kt`

- [ ] **Step 1: 写失败测试 NetGuardEngineTest**

```kotlin
package io.github.yuanbaobaao.petallink.core.net_guard

import kotlin.test.Test
import kotlin.test.assertEquals

class NetGuardEngineTest {
    @Test
    fun 失败立即转OFFLINE() {
        val engine = NetGuardEngine()
        assertEquals(NetState.OFFLINE, engine.onProbeResult(false, gen = 1))
    }

    @Test
    fun 首次成功不转ONLINE_需连续2次() {
        val engine = NetGuardEngine()
        assertEquals(NetState.OFFLINE, engine.onProbeResult(true, gen = 1))  // 第1次成功仍 OFFLINE
        assertEquals(NetState.ONLINE, engine.onProbeResult(true, gen = 1))   // 第2次成功转 ONLINE
    }

    @Test
    fun 成功与失败交替不满足防抖() {
        val engine = NetGuardEngine()
        engine.onProbeResult(true, gen = 1)   // 连续计数=1
        engine.onProbeResult(false, gen = 1)  // 失败立即清零并 OFFLINE
        assertEquals(NetState.OFFLINE, engine.onProbeResult(true, gen = 1))  // 再次第1次成功仍 OFFLINE
    }

    @Test
    fun 代际过期的回调被忽略() {
        val engine = NetGuardEngine()
        engine.onProbeResult(true, gen = 1)   // gen=1, 连续=1
        engine.onProbeResult(true, gen = 2)   // 新代际，重置连续=1
        // gen=1 的迟到回调应被忽略
        assertEquals(NetState.OFFLINE, engine.onProbeResult(true, gen = 1))
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `./gradlew :shared:macosArm64Test --tests "*NetGuardEngineTest" 2>&1 | tail -5`
Expected: FAIL（NetGuardEngine 未定义）

- [ ] **Step 3: 实现 NetState + NetGuardEngine**

```kotlin
package io.github.yuanbaobaao.petallink.core.net_guard

enum class NetState { ONLINE, OFFLINE }

/**
 * 网络探测纯逻辑（对标 src/core/net_guard.rs 防抖逻辑）。
 * 纯状态机，无 IO，可单测。连续 2 次成功才转 ONLINE。
 */
class NetGuardEngine {
    private var current: NetState = NetState.OFFLINE
    private var consecutiveSuccess: Int = 0
    private var currentGen: Int = 0

    fun onProbeResult(success: Boolean, gen: Int): NetState {
        if (gen != currentGen) {
            currentGen = gen
            consecutiveSuccess = 0
        }
        if (success) {
            consecutiveSuccess++
            if (consecutiveSuccess >= 2) current = NetState.ONLINE
        } else {
            consecutiveSuccess = 0
            current = NetState.OFFLINE
        }
        return current
    }

    fun state(): NetState = current
}
```

- [ ] **Step 4: NetGuard expect 接口**

```kotlin
package io.github.yuanbaobaao.petallink.core.net_guard

expect class NetGuard() {
    val state: NetState
    fun startProbe()
    fun stopProbe()
    fun newGeneration(): Int
}
```

- [ ] **Step 5: 运行测试验证通过**

Run: `./gradlew :shared:macosArm64Test --tests "*NetGuardEngineTest" 2>&1 | tail -5`
Expected: PASS（4 tests）

- [ ] **Step 6: 提交**

```bash
git add shared/src/commonMain/.../core/net_guard/ shared/src/commonTest/
git commit -m "feat(stage1): net_guard 纯逻辑 + 2次成功防抖单测"
```

---

## Task 5: config 校验 + 单测（TDD）

**Files:**
- Create: `shared/src/commonMain/kotlin/.../config/UserConfig.kt`
- Create: `shared/src/commonMain/kotlin/.../config/ConfigValidator.kt`
- Create: `shared/src/commonMain/kotlin/.../config/ConfigStore.kt`
- Test: `shared/src/commonTest/kotlin/.../config/ConfigValidatorTest.kt`

- [ ] **Step 1: 写失败测试 ConfigValidatorTest**

测试用例：concurrency=0 失败、concurrency=21 失败、concurrency=6 通过、pollIntervalSec=30 失败、pollIntervalSec=0 通过（禁用）、debounceSec=0 失败、oauthCallbackPort=0 失败、mountDir 空 失败、mountDir="/" 失败、mountDir 含 ".." 失败。

- [ ] **Step 2: 运行验证失败**

Run: `./gradlew :shared:macosArm64Test --tests "*ConfigValidatorTest" 2>&1 | tail -5`
Expected: FAIL

- [ ] **Step 3: 实现 UserConfig + ConfigValidator**

UserConfig data class（mountDir, concurrency=6, pollIntervalSec=60L, debounceSec=3L, oauthCallbackPort）。
ConfigValidator.validate(config): List<String>（返回错误列表，空即合法）。
规则：concurrency ∈ [1,20]；pollIntervalSec==0 或 >=60；debounceSec>=1；port>0；mountDir 非空且非 "/" 且不含 ".."。

- [ ] **Step 4: ConfigStore expect**

```kotlin
expect class ConfigStore() {
    fun load(): UserConfig?
    fun save(config: UserConfig)
}
```

- [ ] **Step 5: 运行验证通过 + 提交**

Run: `./gradlew :shared:macosArm64Test --tests "*ConfigValidatorTest" 2>&1 | tail -5`
Expected: PASS

---

## Task 6: error 序列化 + 单测

**Files:**
- Create: `shared/src/commonMain/kotlin/.../error/ErrorMetadata.kt`
- Create: `shared/src/commonMain/kotlin/.../error/ErrorSerializer.kt`
- Test: `shared/src/commonTest/kotlin/.../error/ErrorSerializerTest.kt`

- [ ] **Step 1: 写失败测试**

测试 toMap 对 AppError.Network / AppError.Remote(500) / AppError.Auth 各返回正确的 kind/message/status。

- [ ] **Step 2: 实现 ErrorMetadata + ErrorSerializer**

ErrorMetadata（retryAfterMs: Long?, requestSemantics: String?, transportKind: String?）。
ErrorSerializer.toMap(error): Map<String,Any?>（kind、message、status?、retryAfterMs?）。

- [ ] **Step 3: 运行验证通过 + 提交**

---

## Task 7: logging 接口 + 实现

**Files:**
- Create: `shared/src/commonMain/kotlin/.../core/logging/LogLevel.kt`
- Create: `shared/src/commonMain/kotlin/.../core/logging/LogRecord.kt`
- Create: `shared/src/commonMain/kotlin/.../core/logging/LogAppender.kt`
- Create: `shared/src/commonMain/kotlin/.../core/logging/Logger.kt`
- Create: `shared/src/macosMain/kotlin/.../core/logging/LoggerImpl.kt`

- [ ] **Step 1: LogLevel enum + LogRecord data class**
- [ ] **Step 2: LogAppender 接口 + Logger expect**
- [ ] **Step 3: macosMain LoggerImpl（console + ringBuffer，文件滚动 TODO 标注）**
- [ ] **Step 4: 编译验证 + 提交**

---

## Task 8: macosMain actual 实现（ConfigStore / NetGuard / repositories / DatabaseDriver）

**Files:**
- Create: `shared/src/macosMain/kotlin/.../config/ConfigStore.kt`
- Create: `shared/src/macosMain/kotlin/.../core/net_guard/NetGuardImpl.kt`
- Create: `shared/src/macosMain/kotlin/.../data/DatabaseDriver.kt`
- Create: `shared/src/macosMain/kotlin/.../data/repository/*RepositoryImpl.kt`（4文件）

- [ ] **Step 1: ConfigStore actual（JSON 文件持久化，用 kotlinx-serialization）**
- [ ] **Step 2: NetGuard actual（TCP 探测 driveapis.cloud.huawei.com.cn:443，3s 超时，30s 间隔）**
- [ ] **Step 3: DatabaseDriver actual（SQLDelight NativeSqliteDriver）**
- [ ] **Step 4: 4 个 RepositoryImpl（委托 SQLDelight 生成的查询）**
- [ ] **Step 5: 编译验证**

---

## Task 9: 阶段 1 整体验证

- [ ] **Step 1: 双 target 编译**
Run: `./gradlew :shared:compileKotlinMacosArm64 :shared:compileKotlinMacosX64`
Expected: BUILD SUCCESSFUL

- [ ] **Step 2: 全部单元测试**
Run: `./gradlew :shared:macosArm64Test`
Expected: 全部 PASS（ConfigValidator / NetGuardEngine / ErrorSerializer / InodeMap）

- [ ] **Step 3: 提交阶段 1**
```bash
git add -A && git commit -m "feat(stage1): 基础设施层完成（config/logging/net_guard/error/data）"
```
