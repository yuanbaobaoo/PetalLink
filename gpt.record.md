# PetalLink GPT 协作记录：同步韧性、断网恢复与状态一致性整改

> 记录日期：2026-07-12 ～ 2026-07-13（Asia/Shanghai）
> 项目目录：`/Users/Shared/codes/personal/petal-link`
> 实施分支：`codex/sync-resilience-state-machine`
> 实施基线：`main` / `573b138`
> 最终业务代码提交：`108107c fix: harden sync recovery and state settlement`
> 本记录目的：保留本次需求来源、华为 API 核对结论、业务问题的前因后果、整体设计决策、实际修改、验证边界和交付状态，供后续维护与追责使用。

## 1. 记录口径

本文件记录当前任务上下文中可追溯的全部用户原话。助手侧不伪造已经无法逐字还原的历史回复，而是根据已落库的设计文档、Git 提交、最终代码和任务过程，记录当时作出的决策与实际动作。

这不是一份只描述最终 commit 的变更日志。本次工作经历了“全项目审查 → 华为 API 协议核对 → 断网与频繁抖动分析 → 状态不一致诊断 → 整体方案确认 → 持续实现 → 最终静态阻断审查 → 安全收口”的完整过程。

## 2. 对话记录与需求演进

### 2.1 用户原话（按时间顺序）

1. “读取下这个项目的所有文档和代码，分析看看有没有什么问题，主要看看业务逻辑是否合理”
2. “现在你需要先去检查一下华为的 API，然后再回头来检查 业务代码的实现是否有问题”
3. “你需要把断网、频繁断网的场景考虑进去，比如，如果正在上传、下载、更新啥的，此时断网、然后网络恢复这种场景，看看”
4. “我还发现一些bug，就是 传输队列中 失败的选项我重试以后，程序主页 还有失败的提示 等等这些状态不一致的问题，你再给我研究下”
5. “针对你刚才收集、诊断的所有信息，我要起全部修改，你给我出一份完整的开发计划”
6. “认可”
7. “可以”
8. “符合，你记住，我让你出的方案是整体方案，不是分阶段方案哈”
9. “确认”
10. “缺热”（按用户原文保留；结合前后文属于方案确认阶段的信息）
11. “确认”
12. “确认”
13. “继续”
14. “我要求你 加快速度，不要做一些无聊的测试，只做必要的测试”
15. “加速，不要再做测试了”
16. “你把本次的修改、调整记录到 gpt.record.md 中，记住，包括我们的对话记录、修改内容、前因后果，都写下来”

### 2.2 助手侧执行与回应摘要

1. 先审查项目文档、Rust 后端、Vue 前端、SQLite 数据模型、同步规划器、传输执行器、网络守卫、华为 Drive API 封装和托盘/主页状态来源。
2. 按用户要求，把华为 API 合同放在业务代码判断之前，核对 Changes、Files、上传、下载、删除、移动等接口的真实约束，再反查现有实现。
3. 将断网问题从“请求失败后重试”扩展为完整故障模型：请求是否到达服务端、响应是否丢失、会话是否仍有效、本地源是否变化、云端版本是否变化、重启后如何恢复。
4. 将“队列重试后主页仍失败”定位为多状态源与非事务结算问题，而不是单纯的前端刷新问题。
5. 形成并由用户多次确认一套整体架构：持久化任务状态机、最后成功同步基线、可信云端 checkpoint、统一 TaskRunner、权威状态 revision 和危险操作复核。
6. 按依赖顺序实施，但没有把最终产品拆成相互独立的临时阶段；所有模块最终接入同一套状态机和结算模型。
7. 用户要求加速后，停止扩展性测试；最后阶段没有运行测试用例，只保留必要的格式检查、差异检查、静态审查和 `cargo check --lib` 编译检查。
8. 在最终提交前又做了一次只读阻断级审查，发现并修复 5 个仍可能造成数据丢失或状态不收敛的问题。
9. 最终业务修改提交为 `108107c`，提交后工作区干净；随后按用户要求新建本记录文件。

## 3. 用户确认的整体约束

本次设计和实现遵守以下已确认约束：

- 必须是整体方案，不接受只修某一个页面提示或某一个断网分支的局部补丁。
- 必须先尊重华为 Drive API 的真实合同，再设计重试与业务收敛逻辑。
- 断网、频繁断网、睡眠/唤醒、请求已提交但响应丢失、进程中断都属于正常运行场景。
- 上传、下载、更新、删除、移动和创建目录都不能因盲目重放造成重复资源或数据丢失。
- `sync_items` 只保存最后一次确认成功的同步基线；失败、等待、歧义和取消不能推进成功基线。
- `transfer_queue` 保存任务生命周期和恢复上下文；人工重试复用原任务 ID，不创建绕过状态机的旁路任务。
- 主页、传输面板、文件状态和托盘必须来自同一份权威状态快照，并用单调 revision 防止旧事件覆盖新状态。
- 云端索引不完整或未追平时，禁止基于“云端不存在”执行删除或覆盖。
- 用户要求停止测试后，不再运行测试；交付时明确保留这一验证边界，不宣称完整测试套件通过。

## 4. 初始问题、根因与风险

### 4.1 失败状态存在多个事实来源

**现象**：传输队列中的失败任务点击重试后，队列可能已经进入重试或成功，但主页仍显示失败；清除队列历史也可能影响或不影响主页提示，行为不一致。

**根因**：

- `transfer_queue`、`sync_items.status/error_message`、前端 Pinia store 和 Tauri 事件各自保存状态。
- 部分事件只是“通知刷新”，部分事件却携带不完整或旧的全局状态。
- 接受重试、任务成功和兼容状态清理不是同一个事务。
- 某些代码按路径或名字更新任务，而不是按任务 ID + revision 结算。

**风险**：用户无法判断当前是否真的失败；旧失败提示可能永久残留；更危险的是 UI 显示成功但数据库仍保留失败或旧基线。

### 4.2 网络错误被当成一种普通失败

**现象**：断网、超时、429、5xx、401、会话过期和业务校验失败可能都落入相似的失败分支。

**根因**：缺少结构化错误类型、持久化重试预算以及 Waiting/Backoff/Verifying 等中间状态。

**风险**：

- 网络恢复后任务不会自动继续。
- 频繁断网造成热循环或重复请求。
- 永久失败被无限重试。
- 请求可能已经到达服务端时再次 POST，制造重复文件或目录。

### 4.3 上传结果不确定时存在盲目重放风险

**现象**：分片上传最后一片、创建上传或更新请求如果响应丢失，客户端无法仅凭本地错误判断服务端是否已经提交。

**根因**：会话 URL、服务端 offset、远端结果 fileId、任务时间窗和源文件快照没有形成统一的持久化核验链。

**风险**：重复文件、错误覆盖、同一失败任务反复创建新资源。

### 4.4 下载可能覆盖用户刚写入的内容

**现象**：下载先写 `.tmp`，最终 rename 会原子替换目标；但如果用户在下载等待期间编辑或新建目标文件，旧实现可能直接覆盖。

**根因**：只校验云端下载版本，没有在安装 `.tmp` 前再次核验本地目标快照。

**风险**：明确的本地数据丢失。

### 4.5 删除、释放空间和路径操作存在 TOCTOU

**现象**：规划阶段确认可删除后，网络核验和实际 unlink 之间可能经过较长时间；用户可在此期间修改文件。

**根因**：危险操作缺少执行前后的版本复核、路径互斥租约、无覆盖 rename 和崩溃恢复标记。

**风险**：删除新内容、覆盖目标路径、进程退出后留下无法判断归属的半完成状态。

### 4.6 云端树可能不完整却被当成事实

**现象**：Changes 分页异常、cursor 循环、全量 BFS 子目录失败或启动时只加载旧缓存时，内存树可能缺少文件。

**根因**：旧实现没有把“可展示的旧树”和“可用于危险规划的可信 checkpoint”严格区分。

**风险**：把未拉到的云端文件误判为已删除，从而删除本地文件或重复上传。

### 4.7 同步触发可能被吞掉

**现象**：Watcher、网络恢复、自动刷新、手动刷新在已有周期运行时同时触发，部分请求可能只发现“正在同步”然后返回。

**根因**：缺少带序列号的请求合并、sticky pending 标记和单 owner drain。

**风险**：断网恢复后没有补扫；新变更长时间不处理；状态停留在 running/indexing。

### 4.8 本地路径和数据库读取存在 fail-open

**现象**：部分 `.ok().flatten()`、`rows.flatten()`、`Path::exists()` 会把权限错误、损坏行或 IO 错误当成“不存在”。

**根因**：为了容错而吞错，但在同步和删除语义里“不知道”被误当成“没有”。

**风险**：漏掉活动任务、制造错误 baseline、触发误删或错误重建。

## 5. 华为 Drive API 核对结论

本次业务整改以项目内已经整理的华为 API 合同为依据，详细接口表见 `docs/api调用整理.md`。影响同步逻辑的主要结论如下。

### 5.1 Changes

- `/changes` 强制要求非空 cursor，不能把无 cursor 请求当成初始查询。
- 初始 cursor 必须通过 `/changes/getStartCursor?fields=*` 获取。
- `nextCursor` 是当前批次的分页 cursor；`newStartCursor` 是追平后的新 checkpoint，两者语义不能混用。
- 空 `changes` 不代表终页；只要有 `nextCursor` 就必须继续。
- 达到页数上限、cursor 重复、cursor 循环、终页缺少 `newStartCursor` 都必须判协议失败，不能提交部分结果。
- “前页有数据、最后一页为空且 `newStartCursor` 等于当前终页 cursor”是可收敛场景；最终修复将“不推进”判断限定为当前终页确实返回变更时。

### 5.2 Files:list 与根目录

- 华为使用 `queryParam='id' in parentFolder` 风格的查询，不按 Google Drive 的普通 parent 参数处理。
- 根目录语义需要统一处理 `root` 与实际 root folder ID，业务层不能随意混用。
- 全分页结果才可用于创建查重、目标冲突判断和云端树构建。

### 5.3 创建目录

- 创建目录是非幂等 POST。
- 请求前必须完整列出目标父目录并做同名目录唯一核验。
- 写请求只接受符合官方合同的成功状态和完整 `File` 响应。
- 响应丢失后必须再次按 parent + name + folder MIME 做唯一核验：唯一匹配视为已提交，零匹配才可稍后重试，多匹配进入歧义，禁止再次 POST。

### 5.4 更新、重命名和移动

- 已有 fileId 的更新不能失败后退化为 Create。
- 移动必须按当前父目录和目标父目录构造 update，并核对最终 `fileName`、唯一 `parentFolder` 和 fileId。
- 响应不确定时通过 GET 同一 fileId 复核；结果确认前不能结算本地路径和 DB 基线。

### 5.5 删除

- 华为删除为回收/软删除语义，并存在最终一致性窗口。
- 写响应丢失后通过 GET/删除核验确认 404 或 recycled 状态。
- LIST 暂时仍返回旧数据时，不能据此恢复刚删除的资源；本地保留近期删除防振荡信息。

### 5.6 分片上传

- 大文件使用 resumable 会话；Location 头中的 session URL 是恢复所需的持久身份。
- HTTP 308 是正常的 “Resume Incomplete”，不是普通错误。
- 恢复前需要查询服务端状态/连续 range，以服务端确认 offset 为准，不能只信本地偏移。
- 即使本地 offset 为 0，只要已有 session URL，也必须先进入原会话核验，不能重新初始化。
- 会话 404/410 或 uploadId 失效不等于目标一定未提交；必须先核验远端目标，确认未提交后才能原子清空旧会话并创建新会话。
- 401 对同一请求只允许刷新认证后安全重放一次。

### 5.7 下载

- 使用 `.tmp` 保存断点内容和旁路版本元数据。
- Range 恢复必须校验 Content-Range、总大小、ETag/editedTime/hash 等版本身份。
- 下载完成后先复核云端版本，再复核本地目标仍符合任务创建时的条件，最后才安装 `.tmp`。

## 6. 经确认的整体架构

### 6.1 三个事实层

1. `transfer_queue`：任务执行事实，保存操作、源快照、云端期望版本、会话、错误、重试时间、远端结果和 revision。
2. `sync_items`：最后一次确认成功的本地—云端同步基线；失败与等待不能推进 mtime、size、fileId 或 editedTime。
3. `SyncGlobalState`：由数据库和少量 runtime 字段统一聚合的 UI 权威快照，带进程级单调 revision。

### 6.2 九态任务状态机

| 状态 | 含义 | 典型后续 |
|---|---|---|
| `Pending` | 已持久化，等待执行 | `Running` |
| `Running` | 后端请求正在执行 | 完成、等待、退避或核验 |
| `WaitingForNetwork` | 连接类失败，等待稳定网络恢复 | 原任务继续 |
| `BackingOff` | 429/可重试 5xx，等待截止时间 | 到期继续 |
| `VerifyingRemote` | 请求可能已提交，禁止盲目重放 | Committed / NotCommitted / Ambiguous |
| `RestartRequired` | 源文件、目标或会话条件变化 | 重新规划但复用身份链 |
| `Completed` | 远端与本地基线均已确认结算 | 终态 |
| `Failed` | 永久失败或需人工处理 | 人工重试复用 task ID |
| `Canceled` | 用户取消 | 终态 |

### 6.3 统一 TaskRunner

- 自动同步、单项重试、全部重试、启动恢复和网络恢复使用同一个执行入口。
- 所有生命周期变更使用 task ID + `state_revision` 乐观并发条件。
- 进度更新、会话更新、成功结算和失败结算不再按路径或名称猜任务。
- 接受人工重试时，任务状态和 `sync_items` 兼容状态在一个事务内更新。

### 6.4 可信云端 checkpoint

- tree、path-to-id、root folder ID 和 Changes cursor 作为一个原子 checkpoint 持久化。
- 旧 checkpoint 可以用于展示和 Changes catch-up，但未追平前 `trusted=false`。
- 全量 BFS 采用 “先取 startCursor → BFS → replay Changes → 原子提交” 消除扫描窗口。
- 任何分页、协议、持久化或 merge 失败都保留旧 checkpoint，同时撤销危险操作授权。

### 6.5 危险操作原则

- 网络或协议“不确定”永远不是删除或覆盖的许可。
- 危险操作在真正副作用前重新检查本地/云端版本。
- 同一路径及祖先/子孙路径使用活动租约，阻止传输、删除、移动和释放空间交叉执行。
- 路径移动使用 no-clobber rename；进程中断后依据 fileId xattr、可信云树和 DB 子树恢复。

## 7. 实施提交时间线

设计与计划：

| 提交 | 时间 | 内容 |
|---|---|---|
| `096f461` | 2026-07-12 19:07 | 写入整体设计 `docs/superpowers/specs/2026-07-12-sync-resilience-and-state-consistency-design.md` |
| `573b138` | 2026-07-12 19:11 | 写入完整实施计划 `docs/superpowers/plans/2026-07-12-sync-resilience-and-state-consistency.md`；该提交也是本次实施分支的 `main` 基线 |

实施提交：

| 提交 | 时间 | 主要内容与因果 |
|---|---|---|
| `bbebc63` | 2026-07-12 20:46 | 新增持久化传输状态模型、schema v5 字段、operation/error/revision，为后续恢复提供数据基础 |
| `37448cd` | 2026-07-12 21:10 | 校验旧任务恢复路径；无法安全推导 mount-relative path 的旧任务不再被猜测执行 |
| `0052e37` | 2026-07-13 07:42 | 新增权威状态聚合器和单调 revision，统一主页、托盘和传输状态的事实来源 |
| `1e7a577` | 2026-07-13 09:20 | 新增结构化恢复错误分类，区分 network、backoff、remote ambiguous、permanent failure 等 |
| `51f1c6b` | 2026-07-13 10:21 | 自动传输与人工重试统一进入 TaskRunner，移除旁路结算逻辑 |
| `8735d78` | 2026-07-13 11:25 | 加固启动恢复和同路径任务仲裁，防止重复 active intent 与过期任务重放 |
| `2eda67c` | 2026-07-13 12:57 | 引入可合并、不会丢失的同步周期请求；断网期间的 watcher/刷新请求恢复后补跑 |
| `b5a5985` | 2026-07-13 14:54 | 收口网络恢复、退避、验证、状态复位等多个边界缺口 |
| `9b1cddd` | 2026-07-13 15:20 | 强制可信华为云端 checkpoint；不完整云树不再授权删除和基线自愈 |
| `c069958` | 2026-07-13 15:27 | 网络/启动恢复前先追平当前云端状态，避免用旧云树恢复并覆盖新远端版本 |
| `ddef85b` | 2026-07-13 15:30 | 下载断点恢复不再提前推进失败基线；失败时保留原文件和最后成功事实 |
| `6f88451` | 2026-07-13 15:47 | 重试、任务结算和主页状态 revision 对齐，解决旧事件覆盖新状态的问题 |
| `40cb6f7` | 2026-07-13 15:56 | 华为写操作结果核验后才执行危险的本地/DB 结算，防止响应丢失后的误删与重复操作 |
| `6fe7b6f` | 2026-07-13 16:09 | 将韧性状态合同写入 README、API 整理和概要设计文档 |
| `108107c` | 2026-07-13 17:36 | 最终整体安全收口：断点会话、目录创建、路径恢复、释放空间、DB fail-closed、删除复核、重试一致性和最终阻断项修复 |

## 8. 最终修改内容（按模块）

### 8.1 数据库与任务模型

涉及：`src/data/migrations.rs`、`src/data/mod.rs`、`src/data/repository.rs`、`src/sync/transfer_state.rs`。

- schema 升级到 v5，保存 relative path、parent ID、operation、源 mtime/size、云端 editedTime、attempt、next retry、error kind、remote result fileId 和 state revision。
- 旧数据库原位迁移，不删除同步基线和传输历史。
- `sync_items.local_path` 明确统一为相对挂载根的规范 UTF-8 路径。
- 状态变更统一做合法转换校验和 revision CAS。
- 读取损坏行不再 `flatten` 跳过；任一活动任务行无法解析时整体失败，避免把“有任务”误读成“空闲”。
- `find_by_file_id` 检测重复 fileId 基线；存在歧义时拒绝猜路径。
- 只有远端明确确认上传未提交后，才在事务内清除失效 session URL、server/upload ID 和 offset。

### 8.2 权威状态与前端一致性

涉及：`src/sync/status_aggregator.rs`、`src/sync/state.rs`、`src/commands.rs`、`src/platform/tray.rs`、`app/stores/sync.ts`、`app/stores/transfer.ts`、`app/api/*.ts`、`app/views/main/*.vue`。

- `SyncGlobalState` 由数据库实时聚合，不复用旧计数。
- revision 来自进程级共享源，即使 SyncEngine 被替换也不会倒退。
- `sync_state` 携带完整权威快照；`transfer_update` 只表示队列需要重载，不能携带一个不完整全局状态覆盖主页。
- 接受重试时，Failed task → Pending 与对应 `sync_items` Failed → Syncing 同事务完成。
- 重试再次失败或成功时，任务与兼容状态同事务结算并重新广播。
- 清理传输历史只删除历史行，不修改仍真实存在的同步失败事实。
- 队列将 WaitingForNetwork、BackingOff、VerifyingRemote、RestartRequired 与永久 Failed 分开展示。

### 8.3 网络守卫与同步周期协调

涉及：`src/core/net_guard.rs`、`src/sync/engine.rs`、`src/mount/local_watcher.rs`。

- 请求失败可以立即产生离线边沿；稳定 Online 后只合并唤醒一次恢复周期。
- 离线时保留同步请求，不把 watcher 变化丢弃。
- 周期请求使用序列和 bit flags 合并，由单 owner drain；运行中到达的新请求在当前周期后补跑。
- 启动、网络恢复、到期退避、手动刷新、自动刷新和本地 watcher 进入同一个协调器。
- 恢复固定顺序：云端 catch-up → VerifyingRemote → WaitingForNetwork → 到期 BackingOff → 本地扫描与 planner。
- 周期提前返回、错误、路径恢复冲突和索引失败时均恢复 runtime idle 字段，避免主页卡在 running/indexing。

### 8.4 华为云树和 Changes

涉及：`src/drive/changes_api.rs`、`src/drive/files_api.rs`、`src/sync/cloud_tree.rs`、`src/sync/engine.rs`。

- Changes 全分页读取，检测循环、停滞、缺失字段和页数上限。
- 最终 cursor 只在完整追平后提交。
- 修复“累计有变更但最后空页 cursor 合法不变”被错误拒绝的问题。
- 全量树、path-to-id、root ID 和 cursor 作为同一个 checkpoint 持久化。
- 不完整结果保留供只读诊断，但 `trusted=false` 时禁止 reconcile、stale purge 和删除规划。

### 8.5 上传与会话恢复

涉及：`src/drive/upload_api.rs`、`src/error.rs`、`src/sync/retry_policy.rs`、`src/sync/task_runner.rs`、`src/sync/executor.rs`。

- 已持久化 session URL 即使 offset=0 也先查询原会话状态。
- 以服务端确认的连续 range 决定恢复 offset。
- session 404/410 映射为结构化 `SessionExpired`，但仍先进入 VerifyingRemote；只有核验 NotCommitted 才清除会话并新建。
- Create 响应丢失后按 parent/name/size/固定任务时间窗及可用 hash 查找唯一候选。
- 核验时间窗锚定任务创建时间，并放宽到可覆盖慢速/多次中断上传的固定 30 天上界，不使用不断向后移动的“当前时间窗”。
- Update 永不降级 Create。
- 上传完成后如果本地源又被编辑，按“真正上传的旧源快照”结算成功基线；下一轮 planner 会把当前新编辑识别为 Update，不再在 VerifyingRemote 与 RestartRequired 之间循环。
- fileId xattr 补写失败不再阻塞已经确认的远端上传结算；DB 成功基线仍可驱动后续更新。

### 8.6 下载与本地目标保护

涉及：`src/drive/download_api.rs`、`src/sync/executor.rs`、`src/sync/task_runner.rs`、`src/commands.rs`。

- `.tmp` 和版本元数据在可恢复网络错误下保留，使用 Range 继续。
- 校验 Content-Range、长度、hash、云端前后版本，版本变化时丢弃不可信断点。
- `DownloadUpdate` 在任务创建时保存目标文件 mtime/size；排队、重试和真正安装前都要求目标仍匹配。
- 普通 Download 只允许安装到“不存在”或“同一 fileId 的未修改占位符”；网络等待期间出现用户文件时拒绝覆盖，并保留用户内容和 `.tmp`。
- 按需下载同样保存目标快照，真实 0 字节已下载文件不再被误判成占位符。
- 冲突下载的目标副本也使用本地目标守卫，防止用户在下载期间创建同名文件后被覆盖。
- 下载失败不推进 `sync_items`；下载完成后才写真实本地 mtime/size 和云端版本。

### 8.7 创建目录和结构动作

涉及：`src/drive/files_api.rs`、`src/sync/executor.rs`、`src/sync/engine.rs`。

- CreateFolder 前后都执行完整父目录唯一查重。
- 严格校验 200 状态、File.id、名称、文件夹 MIME 和唯一 parent。
- 云端目录创建成功但 DB 写入前崩溃时，可信云树 + 同路径同类型本地目录可恢复缺失 baseline。
- 结构动作必须先完成 DB 事务，再更新内存 cloud tree/path map；DB 失败不发布虚假内存成功。
- 普通 Skip 成为真正 no-op，不再把防误删、墓碑或访问异常的 Skip 错写为成功基线。
- 仅 legacy `pending:<path>` 且同路径存在具体云端文件时，才允许 Skip 收敛；还必须复核本地大小和真实元数据。

### 8.8 删除安全

涉及：`src/sync/executor.rs`、`src/mount/manager.rs`、`src/commands.rs`。

- DeleteFromLocal 在 unlink 前要求远端已删除，并递归验证每个本地文件/目录都匹配持久化基线。
- 未知子项、重复路径、符号链接、读取错误或任何版本变化都阻止删除。
- 远端 GET 可能等待网络，因此网络返回后再次完整复核本地快照，再执行删除。
- 真实 0 字节用户文件只要与成功基线匹配即可删除，不再与占位符混淆。
- 直接云端删除在确认远端回收后才处理本地；本地内容变化时保留并交给冲突救援，不因用户点击删除而静默擦除新编辑。

### 8.9 释放本地空间

涉及：`src/commands.rs`、`src/mount/manager.rs`。

- 必须同时满足可信云树、同 fileId、云端 size/editedTime、成功 DB baseline、本地 mtime/size 和无活动任务。
- 原文件先原子移动到 watcher 忽略的同目录 `.hwcloud_freeup-*` staging。
- staging 写入原始 relative path 和 fileId xattr，并在移动前 fsync。
- 严格占位符用 create-new 创建，禁止覆盖已有路径；xattr 初始化失败时删除半成品并恢复原文件。
- DB 使用 CAS 更新为 cloud-only；失败时恢复原文件和原 baseline。
- 进程启动前扫描残留 staging：若 DB 已提交且占位身份匹配则完成清理，否则恢复原文件；目标冲突时以“释放空间恢复-*”可见副本保留内容。

### 8.10 重命名、移动和崩溃恢复

涉及：`src/commands.rs`、`src/sync/path_recovery.rs`、`src/sync/engine.rs`、`src/sync/executor.rs`。

- 远端路径写入前，将 fileId xattr 持久化到本地源 inode。
- 直接重命名/移动对源与目标获取祖先/子孙重叠检测的独占路径租约。
- macOS 使用 `renameatx_np(RENAME_EXCL)`，目标存在时绝不覆盖。
- 本地跨目录移动不再被规划成“上传新文件 + 删除旧云端文件”，而是 `MoveInCloud`，保持同一 fileId，并可同时改名。
- 移动只结算结构事实，保留原内容 mtime/size/hash/status/error；随后立即重新扫描，若内容也变化则单独 Update。
- 启动/周期在可信云树下按唯一 fileId 恢复远端已提交但本地/DB 未结算的路径变更。
- 文件夹恢复会校验整个 DB/云端子树并事务化 re-key；源、目标同时存在或目标身份不匹配时 fail closed。

### 8.11 数据库和文件系统 fail-closed

- `scan_local`、DB snapshot、reconcile、stale purge 和结果结算均传播错误。
- 重复 local path、重复 fileId、损坏任务行和目录读取失败不再被跳过。
- 关键路径由 `symlink_metadata` 区分 NotFound 与其他 IO 错误；权限错误不再等价于“不存在”。
- 占位创建使用 `create_new`，避免检查与创建之间的覆盖竞态。
- 成功结果以一个 SQLite 事务结算；事务提交后才更新内存缓存。

## 9. 最终阻断级静态审查与补救

用户要求“不再做测试”后，最终阶段只做了快速只读静态审查。审查发现以下 5 个阻断项，均在 `108107c` 提交前修复。

| 阻断项 | 原因与后果 | 最终修复 |
|---|---|---|
| DownloadUpdate 覆盖竞态 | 没有保存目标 mtime/size，下载期间用户编辑仍会被 `.tmp` rename 覆盖 | 保存目标快照；排队、重试和安装前复核；变化时保留用户内容与临时下载 |
| DeleteFromLocal 网络等待竞态 | 本地快照只在远端 GET 前验证，GET 返回后直接 unlink | 远端核验后再次递归验证完整本地快照 |
| “重试全部”被 Failed 屏障挡住 | 只把 `sync_items` 改为 Syncing，没有把 Failed task 送入 `prepare_retry`；planner 新 intent 又被 Failed 行阻塞 | 全局重试逐个通过 TaskRunner 接受并执行 Failed task；仍不合法的 Failed 行继续保留真实失败状态 |
| 上传源变化形成状态循环 | 已提交上传因当前本地源变化进入 VerifyingRemote → RestartRequired → VerifyingRemote | 按已经上传的旧源快照结算；当前新内容由下一轮规划为 Update |
| Changes 空终页 cursor 误判 | 使用累计 `all` 判断 cursor 是否推进，前页有变更会污染最后空页判断 | 改为仅依据当前终页 `page_count` 判断同 cursor 是否异常 |

静态审查之后重新执行 `cargo check --lib`，编译通过。

## 10. 变更规模与文件范围

从 `main` / `573b138` 到 `108107c`：

- 42 个文件发生变化。
- 约 23,374 行新增，4,274 行删除。
- 覆盖 Rust 后端、Vue/Pinia 前端、SQLite migration、API 合同、同步设计文档和自动化测试代码。

主要新增文件：

- `src/sync/transfer_state.rs`
- `src/sync/status_aggregator.rs`
- `src/sync/task_runner.rs`
- `src/sync/retry_policy.rs`
- `src/sync/path_recovery.rs`
- `app/api/transfer.contract.test.ts`
- `app/stores/sync.test.ts`
- `app/stores/transfer.test.ts`
- `app/views/main/transfer-state-ui.test.ts`

主要修改文件：

- `src/sync/engine.rs`
- `src/sync/executor.rs`
- `src/sync/planner.rs`
- `src/data/repository.rs`
- `src/data/migrations.rs`
- `src/drive/client.rs`
- `src/drive/changes_api.rs`
- `src/drive/files_api.rs`
- `src/drive/upload_api.rs`
- `src/drive/download_api.rs`
- `src/commands.rs`
- `src/mount/manager.rs`
- `src/core/net_guard.rs`
- `src/platform/tray.rs`
- `app/stores/sync.ts`
- `app/stores/transfer.ts`
- `app/views/main/SyncStatusBar.vue`
- `app/views/main/TransferPopover.vue`
- `README.md`
- `docs/api调用整理.md`
- `docs/概要设计文档.md`

## 11. 关键场景整改后的行为

### 11.1 上传中断网

1. 网络类错误进入 WaitingForNetwork，不推进成功 baseline。
2. 已有 resumable session 和服务端 offset 持久保留。
3. 稳定网络恢复后先追平云端 checkpoint，再查询旧 session 状态。
4. 服务端确认 offset 后继续；会话失效则先核验目标是否已提交。
5. 只有明确 NotCommitted 才清旧 session 并安全重建。

### 11.2 上传最后响应丢失

1. 任务进入 VerifyingRemote，禁止直接 POST。
2. 有 remote result ID 时直接 GET 同一资源；没有时按固定任务身份查唯一候选。
3. 唯一提交结果进入 Completed 并事务结算。
4. 本地文件同时被再次编辑时，旧上传版本先结算，当前编辑再规划 Update。

### 11.3 下载中频繁断网

1. `.tmp` 和版本身份保留。
2. 恢复时校验云端仍是同一版本并使用 Range 继续。
3. 目标文件在等待期间变化时，不安装 `.tmp`，不覆盖用户内容。
4. 下载失败不修改最后成功 baseline。

### 11.4 启动时离线

1. 可以加载旧 checkpoint 用于展示和后续 Changes 基线。
2. checkpoint 在追平前不可信，不恢复危险写操作，不进入删除 planner。
3. STARTUP 请求保持 pending；网络稳定后执行 catch-up 和恢复。

### 11.5 单项失败后点击重试

1. 校验 task ID、revision、operation、源/目标快照和云端版本。
2. 接受后原 Failed 行原地变成 Pending，主页兼容失败状态同事务变成 Syncing。
3. 执行中主页与队列来自同一权威 revision。
4. 成功后任务和 baseline 同事务完成，主页失败提示消失。
5. 再次永久失败则队列与主页同时恢复真实失败，不出现一边成功一边失败。

### 11.6 点击“重试全部”

1. 不再只批量清 `sync_items` 的错误字段。
2. 逐个 Failed task 进入 TaskRunner 的 `prepare_retry` 和统一执行路径。
3. 仍未通过校验的任务保持 Failed，不伪装成正在同步。
4. 没有 transfer task 的结构性失败才由下一次 planner 重建动作。

### 11.7 云端删除后本地文件被修改

1. 执行器核验远端删除事实。
2. 网络返回后再次检查本地完整子树版本。
3. 有新内容则取消删除并保留，后续进入冲突救援或重新规划。

### 11.8 重命名/跨目录移动时崩溃

1. 远端写入前 fileId 已写入本地 inode。
2. 下次可信云树按同一 fileId 识别新云端路径。
3. 若本地仍在旧路径，则 no-clobber rename 到新路径；若已经在新路径，则验证 xattr 身份。
4. DB 子树事务化 re-key；任何目标冲突都停止并保留内容。

### 11.9 释放空间时崩溃

1. 原内容位于带恢复 xattr 的 staging，不会直接丢失。
2. 启动前根据占位符与 DB 是否提交决定完成清理或恢复原文件。
3. 原路径已有新用户内容时不覆盖，旧内容以可见恢复副本保留。

## 12. 验证记录与明确边界

### 12.1 最终阶段实际执行

- `git diff --check`：通过。
- 针对修改文件执行 Rust 格式化：通过。
- `cargo check --lib`：通过。
- 最后一次编译没有 Rust warning；只出现项目既有 build script 提示：从 `.env` 注入华为 client 配置到编译期常量，未在记录中输出任何密钥值。
- 最终提交后 `git status --short` 为空。

### 12.2 未执行事项

用户先要求“只做必要的测试”，随后明确要求“不要再做测试了”。因此最终收口阶段：

- 没有运行 Rust 单元测试或集成测试。
- 没有运行 Vitest 前端测试。
- 没有执行完整端到端断网故障注入。
- 不声明“全部测试通过”。

分支中包含早期实施时新增或调整的测试代码，但这与“最终阶段实际运行过测试”是两回事。后续如需发布，应在获得用户允许后再执行针对状态机、断点上传、下载安装保护、Changes 分页和重试 UI 的必要回归验证。

## 13. 仍然保留的产品边界

- 华为 Drive API 本身具有最终一致性；客户端通过 checkpoint、GET 复核和近期删除守卫降低风险，但不能让服务端变成强一致。
- 无法唯一确认的远端写结果不会自动选择候选，任务会保持 Verifying/Failed 并等待人工处理。
- 本实现不引入远程服务、事件溯源平台或服务端 webhook。
- 大文件已有资源更新仍以项目实际验证过的华为协议能力为界，不允许用 Create 冒充 Update。
- 文件系统外部进程可以在极短窗口内继续修改文件；实现已把复核放到不可逆操作前，并用内部路径租约阻止应用自身并发副作用，但不能对任意第三方进程施加全局文件锁。

## 14. 后续维护必须保持的不变量

1. 不得让 Failed/Waiting/Verifying/RestartRequired 推进最后成功 baseline。
2. 不得按路径或名称结算传输任务；必须使用 task ID + revision。
3. 不得用 `transfer_update` 事件覆盖全局 `sync_state`。
4. 不得在 `cloud_tree_trusted=false` 时规划删除、stale purge 或缺失型 reconcile。
5. 非幂等 POST 响应不确定时不得直接重放。
6. Download 安装前必须同时验证云端版本与本地目标条件。
7. Delete/Free-up/Move/Rename 必须 fail closed；IO 错误不能当成 NotFound。
8. 清理队列历史不得清除仍真实存在的同步失败。
9. 新增操作必须接入 TaskRunner、错误分类、状态聚合和事务结算，不能创建旁路状态机。
10. 修改 Huawei API 封装时，应同步更新 `docs/api调用整理.md` 和 `docs/概要设计文档.md`。

## 15. 最终交付状态

- 分支：`codex/sync-resilience-state-machine`
- 最终业务代码提交：`108107c`
- 提交标题：`fix: harden sync recovery and state settlement`
- 最终提交规模：17 个文件，3,139 行新增，657 行删除；新增 `src/sync/path_recovery.rs`。
- 从实施基线到最终提交的整体规模：42 个文件，23,374 行新增，4,274 行删除。
- 业务代码在最终提交后工作区干净。
- 必要编译检查通过，完整测试按用户要求未运行。
- 本 `gpt.record.md` 是随后按用户要求补充的协作与调整记录。
