# PetalLink 同步韧性与状态一致性整体设计

> 日期：2026-07-12  
> 状态：已确认  
> 范围：华为 Drive API 客户端、同步引擎、传输队列、网络恢复、危险操作、前端状态与测试

## 1. 目标

本设计一次性解决当前审查发现的同步正确性问题，不采用临时补丁或相互割裂的阶段架构。实施可以按依赖顺序推进，但最终必须收敛到同一套持久化任务状态机、同一套成功同步基线和同一套全局状态聚合模型。

系统完成后必须满足：

- 断网和频繁断网不会丢失本地变更、覆盖本地内容或盲目重复云端操作。
- 可恢复错误自动等待并恢复；永久错误提供稳定、准确、可操作的提示。
- 上传、下载、更新、删除、移动等结果不确定时先验证云端结果，再决定完成或重试。
- `sync_items` 只记录最后一次确认成功的同步基线，失败操作不推进基线。
- `transfer_queue` 完整记录任务上下文和生命周期，所有更新严格按 task ID。
- 主页、文件行、传输面板和托盘由同一份权威状态驱动，不再互相矛盾。
- 不完整的云端索引绝不参与删除规划。

## 2. 非目标

- 不重写整个应用为事件溯源架构。
- 不引入新的远程服务或服务端组件。
- 不改变华为帐号授权范围和现有挂载目录产品形态。
- 不在缺乏华为 API 证据时假设大文件原位覆盖协议。
- 不把传输历史清理等同于忽略同步错误。

## 3. 核心架构

采用“持久化任务状态机 + 成功同步基线 + 单一状态聚合器”架构。

```text
Watcher / 手动操作 / 云端 Changes / 全量 BFS
                      │
                      ▼
              Sync Planner（纯规划）
                      │
                      ▼
          Persistent Transfer Task（SQLite）
                      │
                      ▼
       Transfer Executor + Error Classifier
          │              │              │
          ▼              ▼              ▼
 Waiting/Backoff   Verify Remote    Permanent Failure
          │              │              │
          └──────────────┴──────────────┘
                         │
                         ▼
                Transactional Settlement
               ├─ transfer_queue
               ├─ sync_items baseline
               ├─ cloud_tree/path_to_id
               └─ authoritative UI state
```

所有自动同步、手动重试、启动恢复和网络恢复都进入同一个任务执行器。禁止为重试另写旁路上传逻辑。

## 4. 数据模型

### 4.1 `sync_items`

`sync_items` 表示最后一次确认成功的本地—云端同步基线，而不是传输任务状态。

约束：

- `local_path` 永远为相对挂载目录的规范 UTF-8 路径。
- `file_id` 为已确认的真实云端资源 ID；未确认的新建任务不写伪造 fileId。
- `local_mtime`、`local_size`、`cloud_edited_time` 只在操作确认成功后更新。
- 网络失败、超时、401、429、5xx、结果不确定和用户取消均不推进成功基线。
- 兼容期可保留 `status/error_message` 字段，但永久失败的事实来源逐步迁移到任务表；不得把 FAILED 直接改成 SYNCED 来伪装重试。

### 4.2 `transfer_queue`

`transfer_queue` 是任务执行事实来源。除现有字段外增加：

- `relative_path TEXT`：相对挂载目录路径。
- `parent_file_id TEXT`：云端父目录 ID。
- `operation INTEGER`：create、update、download、download_update、delete、move、rename、create_folder。
- `source_mtime INTEGER`、`source_size INTEGER`：创建任务时的本地源快照。
- `expected_cloud_edited_time INTEGER`：规划时观察到的云端版本。
- `attempt_count INTEGER NOT NULL DEFAULT 0`。
- `next_retry_at INTEGER`。
- `error_kind INTEGER`：network、timeout、auth、rate_limit、server、quota、permission、validation、session_expired、remote_ambiguous、local_changed、unknown。
- `remote_result_file_id TEXT`：结果复核时发现的云端资源 ID。
- `state_revision INTEGER NOT NULL DEFAULT 0`：任务状态单调版本。

保留现有绝对 `local_path`、`session_url`、`resume_offset` 等字段。相对路径和绝对路径不得互换使用。

### 4.3 任务状态

统一状态为：

- `Pending`：等待执行。
- `Running`：请求正在进行。
- `WaitingForNetwork`：连接/超时错误，等待网络恢复。
- `BackingOff`：429 或可重试 5xx，等待 `next_retry_at`。
- `VerifyingRemote`：请求可能已在服务端成功，正在复核。
- `RestartRequired`：session 失效或源文件变化，需要重新规划。
- `Completed`：已确认成功并完成结算。
- `Failed`：不可自动恢复的永久失败。
- `Canceled`：用户明确取消。

合法状态转换由单一函数校验；非法转换记录错误且拒绝落库。

### 4.4 数据迁移

数据库 schema 升级时：

- 旧 `PENDING/RUNNING` 保守迁移为 `Pending`，启动恢复后重新验证。
- 旧 `FAILED` 根据已知错误码分类；无法分类时迁移为 `Failed/unknown`。
- 从挂载根和绝对 `local_path` 安全推导 `relative_path`；无法推导的旧任务标记 `Failed/validation`，不猜测上传目录。
- 不删除现有传输历史和同步成功基线。
- 迁移必须幂等，并覆盖从 v1—v4 直接升级的测试。

## 5. 网络与错误处理

### 5.1 网络守卫职责

TCP 探测只作为调度提示，不作为请求成功依据。

- ONLINE 默认状态不授权危险操作。
- offline 时不丢弃 watcher 变更，只设置 `local_rescan_required=true`。
- offline → online 只合并为一次恢复事件，避免抖动风暴。
- 恢复事件立即唤醒 `WaitingForNetwork` 任务，并安排一次本地完整扫描与云端增量刷新。
- 即使 `poll_interval_sec=0`，恢复事件仍必须补跑。
- 网络恢复需要稳定窗口，连续探测成功后再批量唤醒；再次失败则任务回到等待状态。

### 5.2 错误分类

- connect、DNS、TLS 连接失败、请求/流超时：`WaitingForNetwork`。
- 401：刷新 Token 后安全重放一次；再次 401 为 `Failed/auth`。
- 429：读取 Retry-After；没有时使用指数退避加随机抖动。
- 500、502、503、504：`BackingOff`，达到上限后进入 `VerifyingRemote` 或 `Failed/server`，取决于操作是否可能已提交。
- 配额、权限、参数和本地校验错误：立即 `Failed`。
- 404：按操作语义处理，不能统一视为失败或成功。

### 5.3 重试预算

重试预算按任务持久化，不能因重启清零：

- 网络等待不消耗永久失败预算。
- 429/5xx 使用有上限的指数退避。
- 401 仅自动刷新重放一次。
- UI 手动重试创建新 attempt，但复用同一个 task ID 和任务上下文。

## 6. 操作幂等与结果复核

### 6.1 新建上传

- 创建任务时保存 relativePath、parentId、source mtime/size。
- 请求超时或响应丢失后进入 `VerifyingRemote`，按父目录、文件名、size、editedTime/hash 和任务时间窗查询。
- 唯一匹配则完成结算；无匹配才允许重试 POST；多个匹配进入永久冲突，不自动选择。
- 恢复或重试前源文件 mtime/size 已变化时，旧任务进入 `RestartRequired` 并重新规划。

### 6.2 已有文件更新

- 有 fileId 的任务永远是 update，不得在 PATCH 失败后退化为 create。
- PATCH 超时后 GET 原 fileId，比较 editedTime、size/hash，确认是否已提交。
- 只有明确未提交且错误可重试时才重放 PATCH。
- 大文件已有资源更新在华为协议未验证前返回明确的 `UnsupportedUpdate/RestartRequired`，不得新建同名副本。

### 6.3 分片上传

- session URL、服务端确认 offset、parentId 和源快照持久化。
- 每片请求支持 401 刷新后原 Content-Range 重放。
- 响应丢失时先查询 session 状态，以服务端 offset 为准。
- session 404/过期时先查目标文件是否已创建；未创建才建立新 session。
- 最终分片后必须通过响应或状态查询确认真实 fileId，才能 Completed。

### 6.4 下载

- 始终写 `.tmp`，成功校验后原子替换目标文件。
- 下载失败保留原文件和 `sync_items` 成功基线。
- 网络恢复后自动重试；UI 单项重试必须真实支持下载。
- 第一版可以从零重新下载；Range 续传作为同一整体方案的性能能力实现，保存临时文件长度、ETag/editedTime，并在云端版本一致时继续。
- 下载完成后写真实 xattr、真实 mtime/size 和云端 editedTime。

### 6.5 删除、重命名和移动

- 删除超时后 GET fileId；404 或 recycled=true 表示成功。
- 重命名/移动超时后 GET fileId，核对 fileName/parentFolder。
- 结果未确认前不更新成功基线和内存云端树。

## 7. API 与云端索引完整性

### 7.1 Changes

- `list_all_changes` 返回并持久化追平后的最终 `newStartCursor`。
- 检测 cursor 不前进、cursor 循环和响应缺字段。
- 异常时停止增量，清理不可用 cursor，回退完整 BFS。
- merge 全部无法解析时继续使用现有全量回退策略。

### 7.2 Files 分页与 BFS

- 仍有 nextCursor 时达到本地页数保护上限必须返回错误，不能返回截断成功。
- 检测分页 cursor 重复。
- 任一目录分页失败、子树永久失败或响应异常时，本次 BFS 为 incomplete。
- incomplete 结果可用于只读展示诊断，但不得替换可信 cloud tree，也不得进入删除规划。
- `complete=true` 仅在所有目录分页完全结束后原子写入。

### 7.3 查询构造

- 搜索关键词先按华为 queryParam 字符串字面量规则转义，再 URL 编码。
- pageSize、字段名和端点行为用 wiremock 契约测试固定。

## 8. 同步引擎一致性

### 8.1 周期互斥

- 使用异步 Mutex guard 或原子 compare-exchange 实现真正的周期互斥。
- Watcher、手动刷新、自动刷新和网络恢复不能并发进入 diff/execute/apply。
- 多个触发在运行期间到达时合并为一个 `rescan_required`，当前周期结束后补跑一次。

### 8.2 规划与执行

- Planner 只读取可信快照并生成任务意图，不直接修改 DB。
- 不完整 cloud tree 时禁止生成 DeleteFromLocal/DeleteFromCloud。
- 每个 action 分配稳定 action ID；执行结果返回 `(action_id, result)`，panic/取消也保留原下标。
- 同一路径同时只允许一个未结束的互斥操作任务。

### 8.3 事务结算

任务成功后用一个结算边界更新：

1. 校验 task ID、state revision 和源快照。
2. 更新 transfer_queue。
3. 更新 sync_items 成功基线。
4. 更新 cloud_tree/path_to_id 或安排下一次可信刷新。
5. 提交事务后生成权威状态广播。

失败和等待状态只更新任务表，不覆盖成功基线。

## 9. 危险操作

### 9.1 释放空间

最终执行命令内重新验证：

- 路径规范且仍位于挂载目录。
- fileId 仍在可信 cloud tree，并通过 API 复核云端存在。
- 本地真实 mtime/size 与最后成功同步基线一致。
- 没有该路径的上传、更新或 VerifyingRemote 任务。
- 删除后创建占位符与 DB 更新作为一个可恢复操作处理。

前端预检查只负责提示，不能替代执行期检查。

### 9.2 删除与覆盖

- 执行前检查 action 基于的版本仍有效。
- 云端索引不可信或网络结果不确定时禁止本地破坏性删除。
- 目录删除保持祖先去重和本地修改救援规则。

## 10. 单一全局状态

新增唯一的权威聚合入口 `recompute_and_broadcast_state()`：

- 从 sync_items 和 transfer_queue 一次性计算 total、completed、failed、conflict、uploading、downloading、waiting_network 和 failed_items。
- 每份状态带单调 `revision`。
- 所有任务状态变化、清理历史、网络恢复和周期结束后调用。
- 禁止发送 `SyncGlobalState::default()` 充当刷新信号。
- 禁止只刷新部分字段并保留其他过期字段。

前端：

- Pinia 仅接受 revision 不小于当前值的完整状态。
- `transfer_update` 只表示传输列表数据变化；`sync_state` 永远是完整快照。
- “等待网络”不计入永久失败。
- 明确区分“传输历史失败”和“当前同步失败”。
- 清除传输历史不改变同步成功与失败事实。
- 上传和下载失败项只有在后端真实支持时才显示重试按钮。

## 11. 测试策略

### 11.1 纯状态机测试

覆盖所有合法/非法状态转换、错误分类、重试预算、revision 和任务互斥。

### 11.2 数据迁移测试

覆盖 v1—当前版本直接升级、旧绝对路径推导、无法推导路径和幂等迁移。

### 11.3 API 契约测试

使用 wiremock 覆盖：

- Changes 多页追平和最终 cursor。
- cursor 不前进和循环。
- Files 达到页数上限但仍有 nextCursor。
- 上传 401、429、5xx、超时、308 丢失和 session 404。
- PATCH 请求成功但响应断开后的远端复核。
- 下载中断、Range 恢复和云端版本变化。
- 删除、移动、重命名结果不确定后的复核。

### 11.4 引擎集成测试

覆盖：

- offline watcher 事件在恢复后被完整扫描处理。
- poll interval 为 0 时仍恢复。
- 两个同步触发源并发时只执行一个周期并补跑一次。
- 失败动作不推进 sync_items 基线。
- 不完整 cloud tree 不产生删除任务。
- 同一路径多条历史任务只结算当前 task ID。
- App 在各任务状态退出并恢复。

### 11.5 前端测试

使用 Vitest 测试：

- 旧 revision 事件不能覆盖新状态。
- 重试开始、成功、失败后主页和传输面板一致。
- 等待网络与永久失败展示不同。
- 清除历史不清除同步失败。
- 下载和上传重试按钮与后端能力一致。

### 11.6 故障矩阵验收

对上传、下载、更新、删除、移动分别验证：请求前断网、请求已发出后断网、响应丢失、频繁上下线、Token 中途过期、进程退出重启。

## 12. 完成标准

- 所有新增及既有 Rust 测试通过；需要本地端口的测试在允许绑定端口的环境运行。
- `cargo clippy --all-targets -- -D warnings` 通过。
- `npm test` 与 `npm run build` 通过。
- 故障矩阵全部有自动化证据或可重复的 macOS 真机验收记录。
- 文档中的 API、状态表、恢复行为和 UI 文案与实现一致。
- 不存在默认空状态广播、绝对/相对路径混写、按 localPath 批量结算或失败推进成功基线。

