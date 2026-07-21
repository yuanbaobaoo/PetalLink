# 10 · 基于 inode 的文件身份识别方案

> **来源**：petal-link-cmp `docs/plan/10`（2026-07 纳入 Flutter 项目，已从"目标架构/迁移计划"重构为本项目的**现行设计文档**）。
> **状态**：Flutter 项目的**文件身份识别标准方案**（替代 Tauri 原版的 fileId xattr 方案；其余业务逻辑仍以 Tauri 为准）
> **日期**：2026-07-16（原始）；2026-07-21（纳入 Flutter）
> **关联文档**：`04-数据模型与持久化`、`06-同步引擎与传输状态机`、`07-安全与OAuth与占位符`；Flutter 侧落地评估与实施计划见 `docs/review/2026-07-21-inode-scheme-evaluation.md`

---

## 一、为什么取代 xattr 方案

### 1.1 原方案：fileId xattr 是复杂度的根源

Tauri 原项目（及 Flutter 当前实现，迁移前）用 macOS 扩展属性（xattr）`com.hwcloud.fileId` 把云端文件身份写进本地文件，再在数据库（`sync_items`）和磁盘 xattr 之间反复核对、消歧、清理。这套机制存在一个根本缺陷：

> **xattr 会跟随文件复制。** 用户用 `cp` 复制一个文件，副本会带着原文件的 `fileId`，导致"同一个云端身份"同时出现在多个本地文件上。

为了对抗这一个事实，原项目长出了一整套兜底机制：复制消歧、多路径去重、身份验证、副本清理、丢失补写。经梳理，fileId 相关实现约 **660 行**，其中 **约 520 行可在 inode 方案下删除**，20+ 处兜底中 **19 处消失**。

### 1.2 fileId 相关复杂度的完整地图

| 模块 | 对应文档章节 | 现状规模 | inode 方案后 | 说明 |
|---|---|---|---|---|
| ① fileId 写入 | `07` §占位符模型 | ~80 行 | 简化到 ~10 行 | `set_file_id_xattr` 整个删除；占位符创建只写 state |
| ② 重命名检测 | `06` §reconciliation | ~180 行 | 重写为 ~40 行 | 复制消歧分支整段删除 |
| ③ 数据库基线协调 | `06` §reconciliation | ~150 行 | 简化到 ~40 行 | "老路径还在不在"三层关卡删除 |
| ④ 远端路径恢复 | `06` §path_recovery | ~160 行 | 简化到 ~50 行 | 8 个检查删 6 个 |
| ⑤ 冲突/备份清理 | `06` §executor | ~50 行 | 全删 | 3 处 `clear_placeholder_xattr` 调用删除 |
| ⑥ 上传后补写 | `06` §executor | ~40 行 | 全删 | "补写失败但不阻塞"逻辑删除 |
| **合计** | | **~660 行** | **~140 行** | **净减约 520 行** |

### 1.3 根因：一棵树连根拔起

把所有兜底归类，它们全部源自同一个事实——**xattr fileId 跟随复制**：

- **消歧**：哪个是原件？哪个是副本？（`detect_renames` 复制分支、`reconcile_db_records` 三层关卡）
- **去重**：多个路径冒充同一 fileId 怎么办？（`recover_one` 的 fileId 计数检查、云树去重）
- **清理**：副本的 fileId 必须撕掉，否则它冒充原件被删（`clear_placeholder_xattr` 5 处调用）
- **补写**：下载/上传会让 fileId 丢失，得补回来（`set_file_id_xattr`、`set_upload_file_id_if_current`）
- **验证**：每次用 fileId 前都要确认它"还是它"（`recover_one` 5 处所有权检查）

### 1.4 为什么 inode 是更优解

inode 是文件系统给每个文件的内部编号。关键特性：**`mv` 改名时 inode 不变，`cp` 复制时 inode 产生新编号。** 这与 xattr 的行为恰好互补：

| 操作 | xattr(fileId) | inode |
|---|---|---|
| 改名（mv） | ✅ 跟随 | ✅ 不变 |
| 复制（cp） | ❌ 跟随（**bug 源头**） | ✅ 新编号（天然区分） |
| 文件夹改名 | ✅ 跟随 | ✅ 内部文件 inode 都不变 |
| 下载覆盖 | ✅ 跟随 | ❌ 变（需主动处理） |

inode 天然具备我们想要的特性（跟随改名），同时不具备我们讨厌的特性（跟随复制）。这意味着 **"同一身份出现在多处"这件事，在 inode 方案下从结构上就不可能发生**，因此上述整棵兜底之树连根拔起。

---

## 二、改动范围与不变量

### 2.1 放弃什么 / 保留什么

| xattr 键 | 处理 | 理由 |
|---|---|---|
| `com.hwcloud.fileId` | ❌ **删除** | 制造复制 bug 的根源，身份改由 DB inode 映射承担 |
| `com.hwcloud.size` | ❌ **删除** | 冗余信息，`stat` 能读真实大小 |
| `com.hwcloud.state` | ✅ **保留** | 占位符身份标记，跟随复制是合理特性（占位符复制还是占位符） |
| `com.hwcloud.freeUpRelativePath` | ❌ **删除** | 恢复路径改记在 DB（`free_up_staging` 表） |
| `com.apple.FinderInfo` | ✅ **保留** | 纯视觉反馈，灰色标签 |

> **核心原则：精准切除病灶（fileId xattr），保留健康组织（state xattr）。**
>
> **重构后 xattr 仅剩 2 个键**：`com.hwcloud.state`（占位状态）+ `com.apple.FinderInfo`（灰标）。

### 2.2 关键不变量

改动必须维持以下不变量，否则视为引入回归：

1. **占位符判定不变**：`is_placeholder = (xattr state=="placeholder")`。`.gitkeep` 等用户 0 字节文件的保护逻辑完全不动。
2. **重命名能力不退化**：本地改名 → 云端 `Files:update`（MoveInCloud），不退化为删+传。
3. **复制行为正确**：副本作为新文件上传，原件不受影响。
4. **下载/释放空间功能正常**：覆盖占位符、释放空间等破坏性流程仍然安全可回滚。

---

## 三、数据库改动（schemaVersion=6）

### 3.1 新增 `local_inode_map` 表

inode 映射是身份识别的核心数据结构。设计为独立表，与 `sync_items` 解耦（见 `04-数据模型`）：

```sql
CREATE TABLE IF NOT EXISTS local_inode_map (
    inode         INTEGER NOT NULL,
    relative_path TEXT    NOT NULL,
    file_id       TEXT    NOT NULL,
    scanned_at    INTEGER NOT NULL,
    PRIMARY KEY (inode)
);
CREATE INDEX idx_inode_map_path ON local_inode_map(relative_path);
CREATE INDEX idx_inode_map_fid  ON local_inode_map(file_id);
```

字段说明：
- `inode`：文件系统 inode 编号。`stat().st_ino` 在 macOS 返回 `u64`，SQLite `INTEGER` 是 64 位有符号，足以容纳；Kotlin 侧用 `Long` ↔ inode 安全转换（inode 不会超过 `Long.MAX_VALUE`）。
- `relative_path`：相对挂载目录的路径，与 `sync_items.local_path` 同语义。
- `file_id`：云端文件 ID，与 `sync_items.file_id` 对应。
- `scanned_at`：上次扫描到该 inode 的时间戳，用于清理陈旧记录。

**为什么独立建表，而不是直接给 `sync_items` 加 `inode` 列？**
- 解耦：不是所有 `sync_items` 记录都有本地 inode（文件夹、已删除墓碑）。
- 生命周期独立：inode 映射每次扫描重建/更新；`sync_items` 是长期基线。
- 删除方便：整个表可以安全清空重建，不影响同步基线。

### 3.2 新增 `free_up_staging` 表（替代 `XATTR_FREE_UP_RELATIVE_PATH`）

释放空间的中断恢复，从"读暂存文件 xattr"改为"读 DB 记录"（见 `04-数据模型` 释放空间流程、`07-安全` 占位符模型）：

```sql
CREATE TABLE IF NOT EXISTS free_up_staging (
    staging_name   TEXT    NOT NULL PRIMARY KEY,  -- 暂存文件名（如 .hwcloud_freeup-xxxx）
    relative_path  TEXT    NOT NULL,              -- 原始相对路径
    file_id        TEXT    NOT NULL,              -- 云端文件 ID
    source_mtime   INTEGER,                       -- 原文件 mtime（回滚恢复用）
    source_size    INTEGER,                       -- 原文件大小（回滚恢复用）
    created_at     INTEGER NOT NULL
);
```

### 3.3 迁移策略（v5 → v6）

在 `04-数据模型` 的迁移链新增 `upgrade_to_v6`：

```text
1. CREATE 上述两张新表（local_inode_map + free_up_staging）。
2. 不回填历史 inode（启动后首次扫描自动填充）。
3. 不删除旧 xattr（向后兼容；启动后由清理流程逐步移除）。
```

迁移是**纯增量**的，不破坏现有数据。首次启动后，扫描流程会自然填充 `local_inode_map`。

---

## 四、核心流程改动

### 4.1 文件身份识别（新增 `identity` 模块）

新增一个独立模块封装 inode 映射的读写，所有身份查询都走它：

```kotlin
// Kotlin/Dart 示意（对应原 src/sync/identity.rs；CMP 实现见 sync/identity/InodeIdentity.kt）
data class InodeRecord(val inode: Long, val relativePath: String, val fileId: String)

/** 查询某 inode 对应的云端身份（fileId + 上次路径）。用于扫描时识别重命名。 */
fun lookupByInode(conn: Connection, inode: Long): InodeRecord?

/** 下载/释放空间完成后主动更新映射（程序自己操作文件时的确定性记账）。 */
fun upsertMapping(conn: Connection, inode: Long, path: String, fileId: String)

/** 扫描结束后，根据本轮见到的 inode 集合清理陈旧记录。 */
fun purgeMissing(conn: Connection, seenInodes: Set<Long>)
```

**设计要点**：身份查询是只读 DB 操作，不碰文件 xattr，不涉及任何"补写自愈"。

### 4.2 本地扫描（`LocalFileEntry` 新增 inode）

扫描时为每个文件读取 inode（见 `04-数据模型` scanLocal）：

```kotlin
// 示意（Flutter 侧为 manager.dart 的 LocalFileEntry）
data class LocalFileEntry(
    // ... 现有字段 ...
    val inode: Long,  // 新增：来自 stat().st_ino（Flutter 经原生 lstat MethodChannel 获取）
)
```

扫描流程改为两阶段：

```
阶段一：遍历目录，收集 (relative_path, inode, size, mtime, is_placeholder)
阶段二：与 local_inode_map 对比，输出三类结果
        ├── 已知 inode + 路径不变 → 正常文件（无变化）
        ├── 已知 inode + 路径变了 → 重命名候选（交给 detect_moves）
        └── 未知 inode → 新文件（上传候选）
```

### 4.3 重命名检测（重写 `detect_renames` → `detect_moves`）

把原来 180 行的 `detect_renames` 重写为基于 inode 的 `detect_moves`（见 `06-同步引擎`），核心逻辑仅约 40 行：

```text
对每个"未知 inode + 新路径"的文件：
  old = lookupByInode(inode)          # 查 DB: 这个 inode 上次在哪？
  if old 存在 AND old.relative_path != 新路径:
      # 同一个 inode 在新路径出现 = 移动！
      action = MoveInCloud {
          fileId: old.fileId,
          relativePath: 新路径,
          ...
      }
      upsertMapping(inode, 新路径, old.fileId)  # 更新映射
```

**与现状的关键差异**：完全不需要"老路径还在不在"的消歧。因为复制产生新 inode，根本不会匹配上老 inode——副本天然被当新文件处理。

原来 `detect_renames` 中的以下分支**全部删除**：
- 复制检测：老路径还在 + 同 fileId → 剥离新路径 fileId
- 延迟替换源跟踪：`deferred_replacement_sources`
- "旧路径被其他文件占用"分支

### 4.4 数据库基线协调（简化 `reconcile_db_records`）

原来"给无基线的本地文件建基线"要过三道 fileId 关卡，全部是为了对抗"同一 fileId 多处出现"。inode 方案下：

| 原关卡 | inode 方案 |
|---|---|
| 必须有 xattr fileId | ❌ 删除（查 inode 映射即可） |
| 同路径云端 id 必须等于 xattr fileId | ❌ 删除（inode 自然配对） |
| 全表扫描确认无其他路径持同一 fileId | ❌ 删除（inode 主键唯一，不可能重复） |

简化后：本地文件无基线但 inode 映射里有 fileId → 直接用该 fileId 建基线。

### 4.5 远端路径恢复（简化 `recover_one`）

`recover_one` 现含 8+ 个检查，其中 6 个与 fileId 歧义相关。inode 方案下：

| 原检查 | inode 方案 |
|---|---|
| 同一 fileId 不能有多条基线 | ❌ 删除 |
| 读老路径 xattr 验证所有权 | 🔄 改为读 inode 映射 |
| 读新路径 xattr 验证所有权 | 🔄 改为读 inode 映射 |
| `read_file_id` 非 UTF-8 致命错误 | ❌ 删除（不读 xattr 了） |

函数预计从 ~160 行缩减到 ~50 行。

### 4.6 冲突/备份清理（删除 `clear_placeholder_xattr` 大部分调用）

`clear_placeholder_xattr` 现存 5 处调用，目的都是"防止副本冒充原件"。inode 方案下副本产生新 inode，DB 自动当独立文件处理，这些调用**全部删除**。

> `clear_placeholder_xattr` 函数本身可保留（只清 state + FinderInfo），供可选的"副本不传播占位符"策略使用，但这不再是 bug 防御，而是体验决策。

### 4.7 上传/下载后处理（删除补写逻辑）

| 函数 | inode 后 |
|---|---|
| `set_file_id_xattr` | ❌ 删除（fileId 不写文件） |
| `set_upload_file_id_if_current` | ❌ 删除，改为 `upsertMapping` |
| `committed_upload` 补写调用 | ❌ 删除 |
| 下载后 `set_file_id_xattr` | ❌ 删除，改为 `upsertMapping` |

**关键改进**：原来的"补写失败但不阻塞基线结算"（容忍不一致）改为 DB 事务内的 `upsertMapping`（要么成功要么回滚）。不再有"补写失败导致身份丢失"的静默故障。

### 4.8 释放空间流程

Tauri 原方案依赖 `XATTR_FREE_UP_RELATIVE_PATH` 标记暂存文件（见 `04-数据模型` 13 步 TOCTOU、`07-安全` 占位符模型）。改动：

1. **不再写 xattr**，改为在事务内向 `free_up_staging` 表插入一条记录。
2. `recover_interrupted_free_up` 改为读 `free_up_staging` 表，不再扫描暂存文件的 xattr。
3. 回滚恢复逻辑不变，但数据来源从 xattr 改为 DB。

**好处**：暂存文件和恢复记录在同一个 DB 事务内，消除"文件已暂存但 xattr 未写"的窗口。

### 4.9 占位符创建（简化 `create_placeholder_*`）

`create_placeholder_if_needed` / `create_placeholder_strict` 现在写 3 个 xattr（fileId/state/size）。改动后只写 1 个 xattr（state）：

```text
建文件（create_new）→ 写 xattr state="placeholder" → 写 FinderInfo 灰标 → DB 写 inode 映射
```

原来"xattr 写失败就删文件回滚"的原子性保证仍然保留（只是从 3 个 xattr 减到 1 个）。

---

## 五、唯一新增兜底：下载覆盖

诚实地说，inode 方案不是零兜底。唯一剩下的确定性场景是**下载覆盖**：

### 5.1 场景

```
下载流程：占位符(0字节, inode=100) → 删除 → 写入真实内容(新 inode=200)
                                                        ↑
                                              inode 变了，DB 还记着 100
```

### 5.2 处理

下载器在写完文件后，**主动更新** inode 映射（确定性记账，非猜测）：

```kotlin
// 下载完成后（Flutter 侧经 PlatformService.getInodeInfo / 批量 stat 通道获取）
val newInode = Files.getAttribute(path, "unix:ino") as Long
identity.upsertMapping(conn, newInode, relativePath, fileId)
```

### 5.3 与现状的本质区别

| | 现状（fileId xattr） | inode 方案 |
|---|---|---|
| 下载后身份处理 | 补写 xattr fileId | 更新 DB inode 映射 |
| 失败模式 | xattr 写失败，"不阻塞"=静默丢失身份 | DB 事务，要么成功要么回滚 |
| 性质 | 面对未知时的猜测性补救 | 自己操作时的确定性记账 |

---

## 六、注意事项与风险

### 6.1 inode 漂移场景

inode 在"文件不被重建"的前提下稳定，但以下场景会漂移：

| 场景 | inode 变化 | 影响 | 应对 |
|---|---|---|---|
| 正常重启 | ✅ 不变 | 无 | — |
| 下载覆盖 | ❌ 变 | 已处理（见第五节） | 下载器主动更新 |
| 跨卷移动 | ❌ 变 | 退化为删+增 | 可接受 |
| 断电/崩溃恢复 | ⚠️ 偶发变 | 误判为删+增 | 见 6.2 |
| Time Machine 还原 | ❌ 一定变 | 误判为删+增 | 见 6.2 |
| 外接/网络卷（SMB/NFS） | ❌ 不稳定 | 不可靠 | 见 6.4 |

### 6.2 inode 漂移的二级兜底（size + mtime）

为防止断电/还原导致的 inode 漂移误判，在 `detect_moves` 中增加一个二级校验：

```text
检测到"老 inode 消失 + 新文件出现"时：
  if 新文件 size == 老记录 size AND 新文件 mtime == 老记录 mtime:
      # 很可能是 inode 漂移而非真实删除，保守处理：不立即判定为删除
      标记为"疑似漂移"，下一轮再确认
```

这是**唯一保留的模糊性兜底**，且它是可选的保守策略（宁可慢一轮，不要误删）。

### 6.3 数据库丢失的影响

`local_inode_map` 是运行期数据，可随时重建：

| 场景 | 影响 | 恢复 |
|---|---|---|
| 正常运行 | 无 | — |
| `local_inode_map` 清空 | 重命名检测失效一轮 | 首次扫描自动重建 |
| 整个 DB 损坏 | 同步基线丢失 | 退化全量同步（现有行为） |

inode 映射丢失**不会导致数据错误**，只会让本轮重命名退化为删+增，下一轮恢复。

### 6.4 外接/网络卷的限制

**重要前提**：本方案假设同步目录在本地 APFS/HFS+ 卷上。如果用户把同步目录设在外接盘或网络卷（SMB/AFP/NFS），inode 语义不稳定。

应对：
- 文档明确声明仅支持本地卷（现状已是如此，限 macOS）。
- 若检测到网络卷，可在 `scan_recursive` 中记录告警，建议用户迁移。

### 6.5 旧 xattr 的清理

迁移后，用户磁盘上可能残留旧的 `com.hwcloud.fileId` / `com.hwcloud.size` xattr。策略：
- **不主动清理**（避免启动时全盘扫描的开销）。
- 代码中**不再读取**这些 xattr，残留值不影响新逻辑。
- 可在后续版本提供一个可选的"清理旧标记"维护命令。

### 6.6 `is_placeholder_file` 保持不变

占位符判定函数**完全不动**，仍然读 `com.hwcloud.state`。这保证了 `.gitkeep` 保护、FSEvents 监听独立性等现有优势不丢失。

---

## 七、实施阶段建议

建议分阶段推进，每阶段独立可测、可回滚。Flutter 侧的落地评估（改造点映射、风险登记）见 `docs/review/2026-07-21-inode-scheme-evaluation.md`。

### 阶段 1：基础设施（不改行为）
- 新增 `local_inode_map` / `free_up_staging` 表与 v6 迁移。
- 新增 `identity` 模块，先实现 `upsertMapping` / `purgeMissing`（仅写不读）。
- 扫描时为 `LocalFileEntry` 填充 `inode` 字段，并在扫描结束写入 `local_inode_map`。
- **此阶段不读取映射、不改任何同步决策**，纯数据采集。
- **验收**：现有功能无变化，DB 多了两张表且被正确填充。

### 阶段 2：重命名检测切换
- 实现 `detect_moves`，与 `detect_renames` 并行运行（feature flag）。
- 对比两者输出，验证 inode 方案正确识别重命名。
- 切换 planner 使用 `detect_moves`。
- **验收**：重命名场景云端执行 `Files:update`，复制场景副本独立上传。

### 阶段 3：删除旧 fileId 兜底
- 删除 `reconcile_db_records` 的 fileId 三层关卡。
- 删除 `recover_one` 的 6 个 fileId 检查。
- 删除 `clear_placeholder_xattr` 的 4 处副本清理调用。
- 删除 `set_file_id_xattr` / `set_upload_file_id_if_current` 及补写逻辑。
- **验收**：全量回归测试通过，代码量明显下降。

### 阶段 4：释放空间迁移
- `free_up` 流程改用 `free_up_staging` 表，删除 `XATTR_FREE_UP_RELATIVE_PATH`。
- `recover_interrupted_free_up` 改读 DB。
- **验收**：释放空间中断恢复功能正常。

### 阶段 5：占位符创建简化
- `create_placeholder_*` 只写 state xattr。
- 删除 `com.hwcloud.fileId` / `com.hwcloud.size` 常量及写入。
- **验收**：占位符创建、下载覆盖、`.gitkeep` 保护全部正常。

---

## 八、收益总结

| 维度 | 原方案（fileId xattr） | inode 方案 |
|---|---|---|
| fileId 相关代码量 | ~660 行 | ~140 行（**-520 行**） |
| 兜底逻辑数量 | 20+ 处 | 1 处（下载覆盖）+ 1 处可选（漂移校验） |
| 复制 bug | 靠大量兜底防御 | **结构上不可能发生** |
| 重命名检测 | 180 行含复制消歧 | 40 行纯 inode 配对 |
| 身份补写 | 容忍静默失败 | DB 事务确定性 |
| xattr 键数量 | 5 个 | **2 个**（state + FinderInfo） |
| 占位符功能 | 不变 | 不变 |
| `.gitkeep` 保护 | 不变 | 不变 |

核心收益：**把"面对未知时的猜测性兜底"替换为"自己操作时的确定性记账"**，复杂度的性质从模糊性转变为可枚举的确定性。
