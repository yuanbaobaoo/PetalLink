package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.sync.*
import io.github.yuanbaobaoo.petallink.sync.executor.*
import kotlinx.coroutines.*

/**
 * 同步引擎（对标 src/sync/engine/cycle.rs run_coordinated_cycle + engine.rs SyncEngine）。
 * 详见 docs/06 §启动恢复、§engine。
 */
class SyncEngine(
    private val executor: SyncExecutor,
    private val taskRunner: TaskRunner,
    private val cycle: CycleCoordinator,
    private val activity: ActivityTracker,
    private val antiOsc: AntiOscillation,
    private val statusAggregator: StatusAggregator,
    private val db: PetalLinkDb,
    private var cloudCache: CloudTreeCache,
    private var dbBaselines: MutableMap<String, DbBaselineEntry>,
) {
    private val scope = CoroutineScope(Dispatchers.Default + SupervisorJob())

    /**
     * 执行一次协调同步周期：启动恢复、云端刷新与同步主流程
     */
    suspend fun runCoordinatedCycle(
        request: CycleRequest,
        localScan: Map<String, LocalEntry>,
        cloudRefresher: CloudTreeRefresher,
    ): Result<Unit> {
        if (request.contains(CycleRequest.STARTUP)) {
            activity.begin(null)?.close()
            taskRunner.performStartupRecovery {}
        }
        val isStartup = request.contains(CycleRequest.STARTUP)
        if (request.contains(CycleRequest.CLOUD_FULL)) {
            cloudCache = cloudRefresher.refreshFull()
        } else if (request.contains(CycleRequest.CLOUD_INCREMENTAL)) {
            cloudCache = cloudCache.cursor?.takeIf { it.isNotBlank() }
                ?.let { cloudRefresher.refreshIncremental(it) }
                ?: cloudRefresher.refreshFull()
        }
        if (!cloudCache.isTrusted()) {
            cycle.restore(request)
            return Result.failure(AppError.Data("云端树不可信"))
        }
        taskRunner.performStartupRecovery {}
        if (request.contains(CycleRequest.ONLINE_RECOVERY)) {
            taskRunner.performOnlineRecovery()
        }
        return runSyncCycleInner(localScan, isStartup)
    }

    /**
     * 同步周期内核：构建快照、规划并过滤动作、执行后落库并刷新状态
     */
    private suspend fun runSyncCycleInner(
        localScan: Map<String, LocalEntry>, isStartup: Boolean,
    ): Result<Unit> {
        val snapshot = SyncSnapshot(localScan, cloudCache.tree, dbBaselines, cloudCache.isTrusted(), isStartup)
        val actions = ActionPlannerGuards.prepare(snapshot, Planner.plan(snapshot))
        val filtered = antiOsc.filter(actions)
        val results = executor.executeActionsOrdered(filtered, cloudCache.pathToId)

        applyResults(snapshot, filtered, results)

        antiOsc.purgeExpired(nowMs())
        statusAggregator.snapshot(db)
        return Result.success(Unit)
    }

    /**
     * 真实 DB 落库（对标 results.rs apply_results）。
     *
     * 对每个成功动作：
     * - DELETE_FROM_CLOUD / DELETE_FROM_LOCAL / BACKUP_BEFORE_CLOUD_DELETE → 删 DB 行
     * - UPLOAD / DOWNLOAD → Skip（TaskRunner 已处理）
     * - CREATE_PLACEHOLDER / CREATE_FOLDER → 写 SyncItem 基线
     * - SKIP（带 cloudFile）→ 收敛 pending 占位 → 写 SyncItem
     * - 失败（deferred=false）→ 写 error
     */
    private suspend fun applyResults(
        snapshot: SyncSnapshot,
        actions: List<SyncAction>,
        results: List<ActionResult>,
    ) {
        for ((i, action) in actions.withIndex()) {
            if (i >= results.size) continue
            val res = results[i]
            val path = action.relativePath

            if (res.success) {
                when (action.type) {
                    SyncActionType.DELETE_FROM_CLOUD,
                    SyncActionType.DELETE_FROM_LOCAL,
                    SyncActionType.BACKUP_BEFORE_CLOUD_DELETE -> {
                        action.fileId?.let { fid -> db.syncItems.deleteByFileId(fid) }
                        antiOsc.addDeleted(path, nowMs())
                    }
                    SyncActionType.CREATE_PLACEHOLDER,
                    SyncActionType.CREATE_FOLDER -> {
                        upsertSyncItem(path, action.fileId ?: res.cloudFileId, snapshot, isFolder = false)
                    }
                    SyncActionType.SKIP -> {
                        // pending 收敛：cloudFile 存在 → 更新真实 fileId
                        if (action.cloudFile != null) {
                            upsertSyncItem(path, action.cloudFile.id, snapshot, isFolder = action.cloudFile.isFolder())
                        }
                    }
                    SyncActionType.CREATE_CONFLICT_COPY -> {
                        // 冲突副本：本地已重命名，DB 保留原 fileId
                        action.fileId?.let { upsertSyncItem(path, it, snapshot, isFolder = false) }
                    }
                    else -> { /* Upload/Download/Verify: TaskRunner handles DB */ }
                }
            } else if (!res.deferred) {
                // 非延迟失败 → 写 error 到 DB
                action.fileId?.let { fid ->
                    val existing = db.syncItems.findByFileId(fid)
                    if (existing != null) {
                        db.syncItems.updateStatus(existing.fileId, existing.localPath, 4, res.errorMessage)
                    }
                }
            }
        }
    }

    /**
     * 写/更新 SyncItem 基线
     */
    private suspend fun upsertSyncItem(
        path: String, fileId: String?, snapshot: SyncSnapshot, isFolder: Boolean,
    ) {
        if (fileId == null) return
        val local = snapshot.local[path]
        val cloud = snapshot.cloud[path]
        db.syncItems.upsert(SyncItem(
            fileId = fileId,
            localPath = path,
            parentFolderId = cloud?.parent,
            name = path.substringAfterLast('/'),
            isFolder = isFolder,
            size = cloud?.size?.toLongOrNull() ?: local?.size ?: 0L,
            localSize = local?.size,
            sha256 = null,
            localMtime = local?.mtime,
            cloudEditedTime = cloud?.modifiedTime?.toLongOrNull(),
            lastSyncTime = nowMs(),
            status = 0,
            errorMessage = null,
        ))
    }

    /**
     * 当前时间戳（毫秒）
     */
    private fun nowMs(): Long = System.currentTimeMillis()
}
