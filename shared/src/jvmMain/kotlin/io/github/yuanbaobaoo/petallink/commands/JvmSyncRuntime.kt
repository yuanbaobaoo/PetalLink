package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.config.ConfigStore
import io.github.yuanbaobaoo.petallink.config.JvmMountPaths
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.drive.ChangesApi
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.drive.DownloadApi
import io.github.yuanbaobaoo.petallink.drive.FilesApi
import io.github.yuanbaobaoo.petallink.drive.UploadApi
import io.github.yuanbaobaoo.petallink.mount.JvmLocalFileScanner
import io.github.yuanbaobaoo.petallink.mount.JvmLocalWatcher
import io.github.yuanbaobaoo.petallink.mount.JvmPlaceholderManager
import io.github.yuanbaobaoo.petallink.mount.JvmUploadStabilityProbe
import io.github.yuanbaobaoo.petallink.mount.MacXattrAccess
import io.github.yuanbaobaoo.petallink.mount.SkipFilter
import io.github.yuanbaobaoo.petallink.sync.DbBaselineEntry
import io.github.yuanbaobaoo.petallink.sync.LocalEntry
import io.github.yuanbaobaoo.petallink.sync.Planner
import io.github.yuanbaobaoo.petallink.sync.SyncAction
import io.github.yuanbaobaoo.petallink.sync.SyncActionType
import io.github.yuanbaobaoo.petallink.sync.SyncSnapshot
import io.github.yuanbaobaoo.petallink.sync.SyncStatus
import io.github.yuanbaobaoo.petallink.sync.ConflictResolver
import io.github.yuanbaobaoo.petallink.sync.engine.ActionPlannerGuards
import io.github.yuanbaobaoo.petallink.sync.engine.ActivityTracker
import io.github.yuanbaobaoo.petallink.sync.engine.BfsCloudTreeRefresher
import io.github.yuanbaobaoo.petallink.sync.engine.CloudTreeCache
import io.github.yuanbaobaoo.petallink.sync.engine.CycleCoordinator
import io.github.yuanbaobaoo.petallink.sync.engine.CycleRequest
import io.github.yuanbaobaoo.petallink.sync.engine.CycleRequestDispatcher
import io.github.yuanbaobaoo.petallink.sync.engine.JvmCloudTreeCheckpointStore
import io.github.yuanbaobaoo.petallink.sync.engine.JvmFreeUpService
import io.github.yuanbaobaoo.petallink.sync.engine.FilesApiFreeUpVerifier
import io.github.yuanbaobaoo.petallink.sync.engine.RuntimeStatus
import io.github.yuanbaobaoo.petallink.sync.engine.StatusAggregator
import io.github.yuanbaobaoo.petallink.sync.engine.UploadStability
import io.github.yuanbaobaoo.petallink.sync.engine.UploadStabilityProbe
import io.github.yuanbaobaoo.petallink.sync.engine.TransferOperationsImpl
import io.github.yuanbaobaoo.petallink.sync.engine.UploadFailedEvent
import io.github.yuanbaobaoo.petallink.sync.engine.JvmTransferFileStore
import io.github.yuanbaobaoo.petallink.sync.engine.TaskRunner
import io.github.yuanbaobaoo.petallink.sync.engine.toTaskContext
import io.github.yuanbaobaoo.petallink.sync.engine.TaskContext
import io.github.yuanbaobaoo.petallink.sync.engine.TaskDisposition
import io.github.yuanbaobaoo.petallink.sync.engine.JvmRemoteTransferVerifier
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.sync.executor.ActionResult
import io.github.yuanbaobaoo.petallink.sync.executor.SyncExecutor
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull

/**
 * JVM 双向同步周期：可信云树、持久 TaskRunner、安全传输与破坏性动作共用唯一 owner。
 */
class JvmSyncRuntime(
    private val paths: AppPaths,
    private val configStore: ConfigStore,
    private val db: PetalLinkDb,
    private val filesApi: FilesApi,
    private val changesApi: ChangesApi,
    private val uploadApi: UploadApi,
    private val downloadApi: DownloadApi,
    private val status: StatusAggregator,
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
    private val stabilityFactory: () -> (suspend (Path) -> UploadStability) = ::defaultStabilityProbe,
) : SyncCommandPlan {
    private val coordinator = CycleCoordinator()
    private val activity = ActivityTracker()
    private val dispatcher = CycleRequestDispatcher(scope, coordinator, ::runCycle)
    @Volatile private var closed = false
    private val started = AtomicBoolean(false)
    private val reconfiguring = AtomicBoolean(false)
    private val reconfigurationGeneration = AtomicLong(0)
    private val reconfigurationMutex = Mutex()
    private val mutationMutex = Mutex()
    @Volatile private var reconfigurationJob: Job? = null
    private val sourceLock = Any()
    private var watcher: JvmLocalWatcher? = null
    private var watcherJob: Job? = null
    private var timerJob: Job? = null
    private val explicitRetries = ConcurrentHashMap.newKeySet<Long>()
    private val folderSyncRunning = AtomicBoolean(false)
    private val mutableFolderSyncProgress = MutableStateFlow<FolderSyncProgress?>(null)
    private val mutableUploadFailures = MutableSharedFlow<UploadFailedEvent>(extraBufferCapacity = 32)

    /**
     * 目录同步进度流。
     */
    override fun folderSyncProgress(): StateFlow<FolderSyncProgress?> = mutableFolderSyncProgress
    /**
     * 上传失败事件流。
     */
    override fun uploadFailures(): SharedFlow<UploadFailedEvent> = mutableUploadFailures

    /**
     * 提交本地重扫+全量云端刷新周期并等待结果。
     */
    override suspend fun manualRefresh(): AppResult<Unit> = submitAndAwait(
        CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_FULL,
    )

    /**
     * 提交本地重扫+增量云端+重试周期并等待结果。
     */
    override suspend fun retryFailed(): AppResult<Unit> = submitAndAwait(
        CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL + CycleRequest.RETRY,
    )

    /**
     * 记录需重试的 taskId，并提交含重试的同步周期。
     */
    override suspend fun retryTransfer(taskId: Long): AppResult<Unit> {
        explicitRetries += taskId
        return submitAndAwait(
            CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL + CycleRequest.RETRY,
        )
    }

    /**
     * 标记进入重配置中状态并停止同步源。
     */
    override fun prepareConfigurationChange() {
        if (closed) return
        reconfigurationGeneration.incrementAndGet()
        reconfiguring.set(true)
        stopSources()
    }

    /**
     * 在重配置完成后按需清空挂载状态、切换同步源并提交全量周期。
     */
    override fun configurationChanged(previous: UserConfig, current: UserConfig) {
        if (closed) return
        val generation = reconfigurationGeneration.get()
        reconfigurationJob = scope.launch {
            reconfigurationMutex.withLock {
                var ready = false
                try {
                    activity.waitUntilIdle()
                    if (closed || generation != reconfigurationGeneration.get()) return@withLock
                    if (mountIdentity(previous) != mountIdentity(current)) {
                        db.clearMountState()
                        deleteMountCheckpoint(previous)
                        deleteMountCheckpoint(current)
                    }
                    if (started.get()) reconfigureSources()
                    ready = true
                } finally {
                    if (generation == reconfigurationGeneration.get()) {
                        reconfiguring.set(false)
                        if (ready && started.get() && current.mountConfigured && current.mountDir.isNotBlank()) {
                            dispatcher.submit(CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_FULL)
                        }
                    }
                }
            }
        }
    }

    /**
     * 配置保存失败后的回滚：清除重配置标记并按需重配同步源。
     */
    override fun configurationChangeFailed() {
        if (closed) return
        reconfigurationGeneration.incrementAndGet()
        reconfiguring.set(false)
        if (started.get()) reconfigureSources()
    }

    /**
     * 启动同步引擎：重配同步源并提交首个启动周期。
     */
    override fun start() {
        if (closed || !started.compareAndSet(false, true)) return
        if (reconfiguring.get()) return
        reconfigureSources()
        dispatcher.submit(
            CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL +
                CycleRequest.ONLINE_RECOVERY + CycleRequest.STARTUP,
        )
    }

    /**
     * 停止同步引擎并关闭同步源。
     */
    override fun stop() {
        if (!started.compareAndSet(true, false)) return
        stopSources()
    }

    /**
     * 网络恢复后提交本地重扫+增量云端+在线恢复周期。
     */
    override fun networkRecovered() {
        if (!closed && started.get() && !reconfiguring.get()) {
            dispatcher.submit(
                CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL + CycleRequest.ONLINE_RECOVERY,
            )
        }
    }

    /**
     * 在变更互斥锁内独占执行块，期间持有活动 guard 防止周期并发。
     */
    override suspend fun <T> exclusiveMutation(block: suspend () -> T): T = mutationMutex.withLock {
        if (closed) throw AppError.Internal("同步引擎已停止")
        if (reconfiguring.get()) throw AppError.Internal("同步目录正在切换")
        val guard = activity.begin(null) ?: throw AppError.Internal("同步引擎正在关闭")
        try { block() } finally { guard.close() }
    }

    /**
     * 远端写已提交后，触发本地重扫+全量云端周期以重新对账。
     */
    override fun remoteMutationCommitted() {
        if (!closed && started.get() && !reconfiguring.get()) {
            dispatcher.submit(CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_FULL)
        }
    }

    /**
     * 尝试入队目录递归同步；占用标志成功置位后异步执行并提交后续对账周期。
     */
    override fun enqueueFolderSync(folderId: String, relativePath: String): Boolean {
        if (closed || !started.get() || reconfiguring.get() || mutationMutex.isLocked ||
            !folderSyncRunning.compareAndSet(false, true)
        ) return false
        scope.launch {
            try {
                exclusiveMutation { syncFolderSubtree(folderId, relativePath) }
                status.snapshot(db, RuntimeStatus(lastSyncTime = System.currentTimeMillis(), contentChanged = true))
                dispatcher.submit(CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_FULL)
            } catch (_: Throwable) {
                runCatching { status.snapshot(db, RuntimeStatus(lastSyncTime = System.currentTimeMillis())) }
            } finally {
                folderSyncRunning.set(false)
            }
        }
        return true
    }

    /**
     * 按当前配置重建本地 watcher 与定时轮询任务；配置缺失或目录不安全则跳过。
     */
    private fun reconfigureSources() = synchronized(sourceLock) {
        stopSourcesLocked()
        val config = runCatching { configStore.load() }.getOrNull() ?: return@synchronized
        if (!config.mountConfigured || config.mountDir.isBlank()) return@synchronized
        val root = runCatching { JvmMountPaths.resolve(config.mountDir) }.getOrNull()
            ?: return@synchronized
        if (Files.isSymbolicLink(root) || !Files.isDirectory(root, LinkOption.NOFOLLOW_LINKS)) return@synchronized
        val localWatcher = runCatching {
            JvmLocalWatcher(
                root,
                scope,
                config.skipPatterns,
                debounceMs = config.debounceSec * 1_000,
            )
        }.getOrNull() ?: return@synchronized
        watcher = localWatcher
        watcherJob = scope.launch {
            localWatcher.changes.collect {
                if (started.get() && !closed && !reconfiguring.get()) dispatcher.submit(CycleRequest.LOCAL_RESCAN)
            }
        }
        runCatching(localWatcher::start)
        if (config.pollIntervalSec >= 60) {
            timerJob = scope.launch {
                while (isActive && started.get() && !closed) {
                    delay(config.pollIntervalSec * 1_000)
                    if (started.get() && !closed && !reconfiguring.get()) {
                        dispatcher.submit(CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL)
                    }
                }
            }
        }
    }

    /**
     * 在 sourceLock 下停止 watcher 与定时任务（对外入口）。
     */
    private fun stopSources() = synchronized(sourceLock) { stopSourcesLocked() }

    /**
     * 取消 watcher/timer 任务并关闭 watcher（调用方需持有 sourceLock）。
     */
    private fun stopSourcesLocked() {
        watcherJob?.cancel()
        watcherJob = null
        timerJob?.cancel()
        timerJob = null
        watcher?.close()
        watcher = null
    }

    /**
     * 提交同步周期请求并轮询等待对应序列号的执行结果。
     */
    private suspend fun submitAndAwait(request: CycleRequest): AppResult<Unit> {
        if (closed) return AppResult.Err(AppError.Internal("同步引擎已停止"))
        if (reconfiguring.get()) return AppResult.Err(AppError.Internal("同步目录正在切换"))
        val sequence = dispatcher.submit(request)
        while (true) {
            coordinator.resultIfCompleted(sequence)?.let { result ->
                return result.fold(
                    onSuccess = { AppResult.Ok(Unit) },
                    onFailure = { AppResult.Err((it as? AppError) ?: AppError.Internal(it.message ?: "同步周期失败")) },
                )
            }
            delay(10)
        }
    }

    /**
     * 在变更互斥锁内独占执行一个同步周期。
     */
    private suspend fun runCycle(request: CycleRequest): Result<Unit> = mutationMutex.withLock {
        runCycleOwned(request)
    }

    /**
     * 同步周期主体：刷新可信云树、恢复中断传输、扫描本地、规划并执行动作、结算基线。
     */
    private suspend fun runCycleOwned(request: CycleRequest): Result<Unit> {
        if (reconfiguring.get()) return Result.failure(AppError.Internal("同步目录正在切换"))
        val guard = activity.begin(null)
            ?: return Result.failure(AppError.Internal("同步引擎正在关闭"))
        try {
        val result = runCatching {
            check(!closed) { "同步引擎已停止" }
        check(!reconfiguring.get()) { "同步目录正在切换" }
        val config = configStore.load() ?: throw AppError.Data("同步配置不存在")
        if (!config.mountConfigured || config.mountDir.isBlank()) throw AppError.Data("尚未配置挂载目录")
        val root = JvmMountPaths.resolve(config.mountDir)
        if (Files.isSymbolicLink(root) || !Files.isDirectory(root, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("挂载目录不存在或不安全: $root")
        }
        status.snapshot(db, RuntimeStatus(isRunning = true, isIndexing = true, syncPhase = "cloud-refresh"))
        val store = JvmCloudTreeCheckpointStore(paths.cloudTreeCheckpoint(root))
        val refresher = BfsCloudTreeRefresher(filesApi, changesApi, store)
        val loaded = store.loadTrusted()
        val cloud = when {
            request.contains(CycleRequest.CLOUD_FULL) -> refresher.refreshFull()
            request.contains(CycleRequest.CLOUD_INCREMENTAL) && loaded?.cursor != null ->
                refresher.refreshIncremental(loaded.cursor)
            loaded != null -> loaded
            else -> refresher.refreshFull()
        }
        cloud.validateTrusted()
        purgeDeletedTombstones(cloud)

        JvmFreeUpService(
            root, paths, db, JvmPlaceholderManager(root, MacXattrAccess), FilesApiFreeUpVerifier(filesApi),
        ).recoverInterrupted()

        val transferStore = JvmTransferFileStore()
        val stability = stabilityFactory()
        val transferOperations = TransferOperationsImpl(
            uploadApi = uploadApi,
            downloadApi = downloadApi,
            readFileBytes = { Files.readAllBytes(Path.of(it)) },
            writeFileBytes = { path, bytes -> Files.write(Path.of(path), bytes) },
            fileExists = { Files.exists(Path.of(it), LinkOption.NOFOLLOW_LINKS) },
            fileSize = { Files.size(Path.of(it)) },
            uploadStability = UploadStabilityProbe { stability(Path.of(it)) },
            fileStore = transferStore,
            remoteVerification = JvmRemoteTransferVerifier(filesApi, transferStore)::verify,
            deleteRemote = filesApi::deleteFile,
        )
        val taskRunner = TaskRunner(db.transfers, transferOperations, { true }, System::currentTimeMillis)
        if (request.contains(CycleRequest.STARTUP)) {
            taskRunner.performStartupRecovery { resumePendingTransfers(taskRunner) }
        }
        if (request.contains(CycleRequest.RETRY)) {
            val ids = explicitRetries.toList()
            explicitRetries.removeAll(ids.toSet())
            if (ids.isEmpty()) {
                val retryable = db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.Failed) +
                    db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired)
                for (task in retryable) task.id?.let { taskRunner.retryExplicit(it) }
            } else {
                for (id in ids) taskRunner.retryExplicit(id)
            }
        }
        taskRunner.performOnlineRecovery()

        status.snapshot(db, RuntimeStatus(isRunning = true, syncPhase = "local-scan"))
        val localEntries = JvmLocalFileScanner(root, MacXattrAccess, config.skipPatterns).scan()
        val local = localEntries.associate { entry ->
            entry.relativePath to LocalEntry(
                entry.relativePath, entry.mtime, entry.size, entry.isPlaceholder, entry.isDirectory,
            )
        }
        val baselines = db.syncItems.selectAll().associate { item ->
            item.localPath to DbBaselineEntry(
                item.fileId, item.localMtime, item.localSize, item.cloudEditedTime, item.status, item.isFolder,
            )
        }
        val snapshot = SyncSnapshot(local, cloud.tree, baselines, cloudTreeTrusted = true, isStartupResume = request.contains(CycleRequest.STARTUP))
        val actions = ActionPlannerGuards.prepare(snapshot, Planner.plan(snapshot))
        val placeholder = JvmPlaceholderManager(root, MacXattrAccess)
        val executor = SyncExecutor(config.concurrency) { action ->
            executeAction(root, placeholder, taskRunner, transferStore, action)
        }
        val results = executor.executeActionsOrdered(actions, cloud.pathToId)

        // 远端写先合并进同一 checkpoint，再提交 DB 基线。
        val committedCloud = commitRemoteWrites(store, cloud, actions, results)
        settleBaselines(root, committedCloud, actions, results)
        val failed = results.withIndex().filter { !it.value.success && !it.value.deferred }
        status.snapshot(db, RuntimeStatus(lastSyncTime = System.currentTimeMillis(), contentChanged = actions.isNotEmpty()))
            if (failed.isNotEmpty()) throw AppError.Data("同步周期有 ${failed.size} 个动作失败")
        }
        if (result.isFailure && !closed) {
            runCatching {
                status.snapshot(db, RuntimeStatus(lastSyncTime = System.currentTimeMillis()))
            }
        }
        return result
        } finally {
            guard.close()
        }
    }

    /**
     * 执行单个同步动作（创建占位符/目录、上传/下载、本地/云端删除、云端移动、冲突副本等）。
     */
    private suspend fun executeAction(
        root: Path,
        placeholder: JvmPlaceholderManager,
        taskRunner: TaskRunner,
        transferStore: JvmTransferFileStore,
        action: SyncAction,
    ): ActionResult {
        return try {
            when (action.type) {
            SyncActionType.CREATE_PLACEHOLDER -> {
                placeholder.createPlaceholderIfNeeded(action.relativePath)
                ActionResult(true)
            }
            SyncActionType.CREATE_FOLDER -> if (action.cloudFile != null) {
                ensureLocalDirectory(root, action.relativePath)
                ActionResult(true)
            } else {
                val file = filesApi.createFile(action.relativePath.substringAfterLast('/'), action.parentFileId, true)
                ActionResult(true, cloudFileId = file.id, cloudFile = file)
            }
            SyncActionType.UPLOAD -> {
                executeTransferAction(root, placeholder, taskRunner, transferStore, action)
            }
            SyncActionType.BACKUP_BEFORE_CLOUD_DELETE -> {
                placeholder.backupModifiedPlaceholder(safeLocalPath(root, action.relativePath).toString())
                ActionResult(true)
            }
            SyncActionType.SKIP -> ActionResult(true)
            SyncActionType.DOWNLOAD -> executeTransferAction(root, placeholder, taskRunner, transferStore, action)
            SyncActionType.DELETE_FROM_CLOUD -> {
                val id = action.fileId ?: throw AppError.Data("云端删除缺少 fileId")
                filesApi.deleteFile(id)
                ActionResult(true)
            }
            SyncActionType.DELETE_FROM_LOCAL -> {
                placeholder.deleteLocal(safeLocalPath(root, action.relativePath).toString())
                ActionResult(true)
            }
            SyncActionType.MOVE_IN_CLOUD -> {
                val id = action.fileId ?: throw AppError.Data("云端移动缺少 fileId")
                var remote = filesApi.getFile(id)
                val desiredName = action.relativePath.substringAfterLast('/')
                if (remote.name != desiredName) remote = filesApi.updateFile(id, desiredName)
                val desiredParent = action.parentFileId
                val currentParent = runCatching { io.github.yuanbaobaoo.petallink.drive.DriveParsers.singleParent(remote) }.getOrNull()
                if (!desiredParent.isNullOrBlank() && currentParent != desiredParent) {
                    if (currentParent.isNullOrBlank()) throw AppError.Data("云端移动缺少旧 parent")
                    remote = filesApi.moveFile(id, currentParent, desiredParent)
                }
                ActionResult(true, cloudFileId = id, cloudFile = remote)
            }
            SyncActionType.CREATE_CONFLICT_COPY -> {
                val source = safeLocalPath(root, action.relativePath)
                val backup = allocateConflictBackup(source)
                Files.move(source, backup, StandardCopyOption.ATOMIC_MOVE)
                executeTransferAction(root, placeholder, taskRunner, transferStore, action.copy(type = SyncActionType.DOWNLOAD))
            }
            }
        } catch (error: Throwable) {
            if (action.type == SyncActionType.UPLOAD) {
                mutableUploadFailures.emit(UploadFailedEvent(action.relativePath, error.message ?: "上传失败"))
            }
            ActionResult(false, errorMessage = error.message)
        }
    }

    /**
     * 执行上传/下载传输动作：复用活动任务或新建传输任务并运行至完成。
     */
    private suspend fun executeTransferAction(
        root: Path,
        placeholder: JvmPlaceholderManager,
        taskRunner: TaskRunner,
        transferStore: JvmTransferFileStore,
        action: SyncAction,
    ): ActionResult {
        val destination = safeLocalPath(root, action.relativePath)
        val direction = if (action.type == SyncActionType.UPLOAD) TransferDirection.UPLOAD else TransferDirection.DOWNLOAD
        val total = if (direction == TransferDirection.UPLOAD) Files.size(destination) else action.cloudFile?.sizeBytes ?: 0L
        val source = if (direction == TransferDirection.UPLOAD) transferStore.snapshot(destination.toString()) else null
        val active = activeTransfer(action.relativePath, direction)
        val id = active?.id ?: db.transfers.insert(
            TransferTask(
                id = null,
                direction = direction,
                fileId = action.fileId,
                localPath = destination.toString(),
                name = destination.fileName.toString(),
                totalSize = total,
                state = io.github.yuanbaobaoo.petallink.sync.TransferState.Pending,
                errorMessage = null,
                createdAt = System.currentTimeMillis(),
                relativePath = action.relativePath,
                parentFileId = action.parentFileId,
                operation = when {
                    direction != TransferDirection.UPLOAD -> 2
                    action.fileId == null -> 0
                    else -> 1
                },
                sourceMtime = source?.modifiedAtMillis,
                sourceSize = source?.size,
                expectedCloudEditedTime = action.cloudFile?.editedTime?.let(::parseEditedTimeMillis),
            ),
        )
        val current = db.transfers.findById(id)!!
        val disposition = when (current.state) {
            io.github.yuanbaobaoo.petallink.sync.TransferState.Pending,
            io.github.yuanbaobaoo.petallink.sync.TransferState.WaitingForNetwork,
            io.github.yuanbaobaoo.petallink.sync.TransferState.BackingOff -> taskRunner.runExpected(current.toTaskContext())
            io.github.yuanbaobaoo.petallink.sync.TransferState.Completed -> TaskDisposition.COMPLETED
            io.github.yuanbaobaoo.petallink.sync.TransferState.Failed -> TaskDisposition.FAILED
            io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired -> TaskDisposition.RESTART_REQUIRED
            io.github.yuanbaobaoo.petallink.sync.TransferState.VerifyingRemote -> TaskDisposition.VERIFYING_REMOTE
            io.github.yuanbaobaoo.petallink.sync.TransferState.Running -> TaskDisposition.BLOCKED
            io.github.yuanbaobaoo.petallink.sync.TransferState.Canceled -> TaskDisposition.CANCELED
        }
        if (disposition != TaskDisposition.COMPLETED) {
            return ActionResult(
                false,
                deferred = disposition in setOf(
                    TaskDisposition.BACKING_OFF, TaskDisposition.WAITING_FOR_NETWORK,
                    TaskDisposition.VERIFYING_REMOTE, TaskDisposition.RESTART_REQUIRED,
                ),
                errorMessage = db.transfers.findById(id)?.errorMessage ?: disposition.name,
            )
        }
        if (direction != TransferDirection.UPLOAD) {
            placeholder.markDownloaded(destination.toString())
            return ActionResult(true)
        }
        val remoteId = db.transfers.findById(id)?.remoteResultFileId
            ?: return ActionResult(false, errorMessage = "上传完成但缺少 remote result")
        val remote = DriveFile(
            id = remoteId,
            name = destination.fileName.toString(),
            size = total.toString(),
            parentFolder = action.parentFileId?.let(::listOf),
        )
        return ActionResult(true, cloudFileId = remoteId, cloudFile = remote)
    }

    /**
     * 选定云端目录的后台 BFS 双向同步；文件下载为真实内容而非占位符。
     */
    private suspend fun syncFolderSubtree(folderId: String, relativePath: String) {
        require(folderId.isNotBlank()) { "目录 fileId 不能为空" }
        val config = configStore.load() ?: throw AppError.Data("同步配置不存在")
        if (!config.mountConfigured || config.mountDir.isBlank()) throw AppError.Data("尚未配置挂载目录")
        val root = JvmMountPaths.resolve(config.mountDir).toRealPath()
        val base = if (relativePath.isBlank()) root else {
            val path = Path.of(relativePath)
            require(!path.isAbsolute && path.none { it.toString() == "." || it.toString() == ".." }) {
                "目录同步相对路径不合法"
            }
            ensureLocalDirectory(root, relativePath)
            safeLocalPath(root, relativePath)
        }
        if (Files.isSymbolicLink(base) || !Files.isDirectory(base, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("目录同步目标不安全: $base")
        }

        val queue = mutableListOf(folderId to "")
        val cloudFiles = linkedMapOf<String, DriveFile>()
        val cloudFolders = linkedMapOf<String, DriveFile>()
        val folderIds = mutableMapOf("" to folderId)
        while (queue.isNotEmpty()) {
            val (parentId, parentPath) = queue.removeAt(0)
            for (file in filesApi.listAllFiles(parentId)) {
                val name = file.name ?: throw AppError.Data("云端子项缺少名称")
                if (SkipFilter.shouldSkip(name, config.skipPatterns)) continue
                require(name.isNotBlank() && name != "." && name != ".." && '/' !in name) { "云端子项名称不合法" }
                val id = file.id ?: throw AppError.Data("云端子项缺少 fileId")
                val subPath = if (parentPath.isEmpty()) name else "$parentPath/$name"
                if (io.github.yuanbaobaoo.petallink.drive.DriveParsers.isFolderMime(file.mimeType)) {
                    cloudFolders[subPath] = file
                    folderIds[subPath] = id
                    queue += id to subPath
                } else cloudFiles[subPath] = file
            }
        }

        for (subPath in cloudFolders.keys.sortedBy { it.count { char -> char == '/' } }) {
            val full = joinRelative(relativePath, subPath)
            ensureLocalDirectory(root, full)
        }
        val localEntries = JvmLocalFileScanner(base, MacXattrAccess, config.skipPatterns).scan()
        val localBySubPath = localEntries.associateBy { it.relativePath }

        // 本地独有目录先在云端按深度补齐，保留层级。
        val missingFolders = localEntries.filter { it.isDirectory && it.relativePath !in cloudFolders }
            .sortedBy { it.relativePath.count { char -> char == '/' } }
        for (entry in missingFolders) {
            val parentSub = entry.relativePath.substringBeforeLast('/', "")
            val parentId = folderIds[parentSub] ?: throw AppError.Data("本地目录父级缺少云端 fileId: ${entry.relativePath}")
            val remote = filesApi.createFile(entry.relativePath.substringAfterLast('/'), parentId, true)
            folderIds[entry.relativePath] = remote.id ?: throw AppError.Data("创建云端目录后缺少 fileId")
        }

        val transferStore = JvmTransferFileStore()
        val stability = stabilityFactory()
        val operations = TransferOperationsImpl(
            uploadApi = uploadApi,
            downloadApi = downloadApi,
            readFileBytes = { Files.readAllBytes(Path.of(it)) },
            writeFileBytes = { path, bytes -> Files.write(Path.of(path), bytes) },
            fileExists = { Files.exists(Path.of(it), LinkOption.NOFOLLOW_LINKS) },
            fileSize = { Files.size(Path.of(it)) },
            uploadStability = UploadStabilityProbe { stability(Path.of(it)) },
            fileStore = transferStore,
            remoteVerification = JvmRemoteTransferVerifier(filesApi, transferStore)::verify,
            deleteRemote = filesApi::deleteFile,
        )
        val runner = TaskRunner(db.transfers, operations, { true }, System::currentTimeMillis)
        val placeholder = JvmPlaceholderManager(root, MacXattrAccess)
        val actions = mutableListOf<SyncAction>()
        val results = mutableListOf<ActionResult>()

        val downloads = cloudFiles.filter { (subPath, _) ->
            val local = localBySubPath[subPath]
            local == null || local.isPlaceholder
        }
        val uploads = localEntries.filter { !it.isDirectory && !it.isPlaceholder && it.relativePath !in cloudFiles }
        val total = downloads.size + uploads.size
        var done = 0
        mutableFolderSyncProgress.value = FolderSyncProgress(done, total)
        for ((subPath, cloudFile) in downloads) {
            val action = SyncAction(
                SyncActionType.DOWNLOAD,
                joinRelative(relativePath, subPath),
                cloudFile.id,
                cloudFile,
                "folder-recursive-download",
                cloudFile.singleParentOrNull,
            )
            actions += action
            results += executeTransferAction(root, placeholder, runner, transferStore, action)
            mutableFolderSyncProgress.value = FolderSyncProgress(++done, total)
        }
        for (local in uploads) {
            val parentSub = local.relativePath.substringBeforeLast('/', "")
            val action = SyncAction(
                SyncActionType.UPLOAD,
                joinRelative(relativePath, local.relativePath),
                reason = "folder-recursive-upload",
                parentFileId = folderIds[parentSub] ?: folderId,
            )
            actions += action
            results += executeTransferAction(root, placeholder, runner, transferStore, action)
            mutableFolderSyncProgress.value = FolderSyncProgress(++done, total)
        }
        val failures = results.filter { !it.success && !it.deferred }
        val tree = cloudFiles.mapKeys { (subPath, _) -> joinRelative(relativePath, subPath) }.toMutableMap()
        actions.zip(results).forEach { (action, result) ->
            result.cloudFile?.let { tree[action.relativePath] = it }
        }
        val cache = CloudTreeCache(
            tree = tree,
            pathToId = tree.mapValues { it.value.id.orEmpty() },
            rootFolderId = null,
            cursor = "folder-sync",
            complete = true,
        )
        settleBaselines(root, cache, actions, results)
        if (failures.isNotEmpty()) throw AppError.Data("目录同步有 ${failures.size} 个任务失败")
    }

    /**
     * 拼接目录同步的相对路径（忽略空白片段，统一以 '/' 分隔）。
     */
    private fun joinRelative(base: String, child: String): String = when {
        base.isBlank() -> child
        child.isBlank() -> base
        else -> "${base.trimEnd('/')}/$child"
    }

    /**
     * 恢复所有处于 Pending 状态的传输任务。
     */
    private suspend fun resumePendingTransfers(taskRunner: TaskRunner) {
        for (task in db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.Pending)) {
            taskRunner.runExpected(task.toTaskContext())
        }
    }

    /**
     * 查找同一相对路径与方向下最近的活动传输任务，用于复用而非重复建任务。
     */
    private suspend fun activeTransfer(relativePath: String, direction: TransferDirection): TransferTask? {
        val states = listOf(
            io.github.yuanbaobaoo.petallink.sync.TransferState.Pending,
            io.github.yuanbaobaoo.petallink.sync.TransferState.Running,
            io.github.yuanbaobaoo.petallink.sync.TransferState.WaitingForNetwork,
            io.github.yuanbaobaoo.petallink.sync.TransferState.BackingOff,
            io.github.yuanbaobaoo.petallink.sync.TransferState.VerifyingRemote,
            io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired,
        )
        val matches = mutableListOf<TransferTask>()
        for (state in states) {
            matches += db.transfers.selectByState(state).filter {
                it.relativePath == relativePath && it.direction == direction
            }
        }
        return matches.maxWithOrNull(compareBy<TransferTask> { it.createdAt }.thenBy { it.id ?: 0L })
    }

    /**
     * 将 ISO-8601 时间字符串解析为毫秒时间戳。
     */
    private fun parseEditedTimeMillis(raw: String): Long = Instant.parse(raw).toEpochMilli()

    /**
     * 把成功的远端写（删除/移动/上传/建目录）合并进云树 checkpoint 并持久化。
     */
    private suspend fun commitRemoteWrites(
        store: JvmCloudTreeCheckpointStore,
        cloud: CloudTreeCache,
        actions: List<SyncAction>,
        results: List<ActionResult>,
    ): CloudTreeCache {
        val tree = cloud.tree.toMutableMap()
        val index = cloud.pathToId.toMutableMap()
        var changed = false
        actions.zip(results).forEach { (action, result) ->
            if (!result.success) return@forEach
            val remote = result.cloudFile
            if (action.type == SyncActionType.DELETE_FROM_CLOUD) {
                val prefix = "${action.relativePath}/"
                val removed = tree.keys.filter { it == action.relativePath || it.startsWith(prefix) }
                removed.forEach { path -> tree.remove(path); index.remove(path) }
                changed = removed.isNotEmpty() || changed
            } else if (action.type == SyncActionType.MOVE_IN_CLOUD && remote?.id != null) {
                val oldPath = index.entries.firstOrNull { it.value == remote.id }?.key
                if (oldPath != null && oldPath != action.relativePath) {
                    val descendants = tree.keys.filter { it == oldPath || it.startsWith("$oldPath/") }.sortedBy { it.length }
                    descendants.forEach { path ->
                        val suffix = path.removePrefix(oldPath)
                        val newPath = action.relativePath + suffix
                        val file = tree.remove(path)!!
                        index.remove(path)
                        tree[newPath] = if (path == oldPath) remote else file
                        index[newPath] = file.id!!
                    }
                    changed = true
                }
            } else if (remote?.id != null && action.type in setOf(SyncActionType.UPLOAD, SyncActionType.CREATE_FOLDER)) {
                tree[action.relativePath] = remote
                index[action.relativePath] = remote.id
                changed = true
            }
        }
        if (!changed) return cloud
        val candidate = CloudTreeCache.trusted(tree, index, cloud.rootFolderId, cloud.cursor!!)
        store.persist(candidate)
        return candidate
    }

    /**
     * 根据动作执行结果更新 DB 同步基线：删除项移除，成功项 upsert 为 SYNCED。
     */
    private suspend fun settleBaselines(
        root: Path,
        cloud: CloudTreeCache,
        actions: List<SyncAction>,
        results: List<ActionResult>,
    ) {
        actions.zip(results).forEach { (action, result) ->
            if (!result.success) return@forEach
            if (action.type == SyncActionType.DELETE_FROM_CLOUD || action.type == SyncActionType.DELETE_FROM_LOCAL) {
                action.fileId?.let { db.syncItems.deleteByFileId(it) }
                    ?: db.syncItems.deleteByLocalPath(action.relativePath)
                return@forEach
            }
            if (action.type == SyncActionType.MOVE_IN_CLOUD) {
                action.fileId?.let { db.syncItems.deleteByFileId(it) }
            }
            val remote: DriveFile = result.cloudFile ?: action.cloudFile ?: cloud.tree[action.relativePath] ?: return@forEach
            val fileId = remote.id ?: return@forEach
            val localPath = safeLocalPath(root, action.relativePath)
            val exists = Files.exists(localPath, LinkOption.NOFOLLOW_LINKS)
            db.syncItems.upsert(SyncItem(
                fileId = fileId,
                localPath = action.relativePath,
                parentFolderId = remote.singleParentOrNull,
                name = remote.name ?: action.relativePath.substringAfterLast('/'),
                isFolder = Files.isDirectory(localPath, LinkOption.NOFOLLOW_LINKS),
                size = remote.sizeBytes,
                localSize = if (exists && Files.isRegularFile(localPath, LinkOption.NOFOLLOW_LINKS)) Files.size(localPath) else null,
                sha256 = remote.contentHash,
                localMtime = if (exists) Files.getLastModifiedTime(localPath, LinkOption.NOFOLLOW_LINKS).toMillis() else null,
                cloudEditedTime = remote.editedTime?.let { runCatching { Instant.parse(it).toEpochMilli() }.getOrNull() },
                lastSyncTime = System.currentTimeMillis(),
                status = SyncStatus.SYNCED,
                errorMessage = null,
            ))
        }
    }

    /**
     * 在挂载根下逐段创建目录；遇符号链接或非目录类型则拒绝。
     */
    private fun ensureLocalDirectory(root: Path, relativePath: String) {
        val target = safeLocalPath(root, relativePath)
        var current = root.toRealPath()
        for (segment in root.relativize(target)) {
            current = current.resolve(segment)
            if (Files.exists(current, LinkOption.NOFOLLOW_LINKS)) {
                if (Files.isSymbolicLink(current) || !Files.isDirectory(current, LinkOption.NOFOLLOW_LINKS)) {
                    throw AppError.LocalIo("目录路径不安全: $current")
                }
            } else Files.createDirectory(current)
        }
    }

    /**
     * 清理 DB 中已不在云树里的 DELETED 墓碑同步项。
     */
    private suspend fun purgeDeletedTombstones(cloud: CloudTreeCache) {
        val liveIds = cloud.tree.values.mapNotNull(DriveFile::id).toHashSet()
        for (item in db.syncItems.selectByStatus(SyncStatus.DELETED)) {
            if (item.fileId !in liveIds) db.syncItems.deleteByFileId(item.fileId)
        }
    }

    /**
     * 校验相对路径并将它解析为挂载根内的绝对路径，拒绝绝对路径、越界与符号链接。
     */
    private fun safeLocalPath(root: Path, relativePath: String): Path {
        val relative = Path.of(relativePath)
        if (relative.isAbsolute || relative.none() || relative.any { it.toString() == ".." || it.toString() == "." }) {
            throw AppError.LocalIo("非法同步路径: $relativePath")
        }
        val canonicalRoot = root.toRealPath()
        val target = canonicalRoot.resolve(relative).normalize()
        if (!target.startsWith(canonicalRoot) || target == canonicalRoot) throw AppError.LocalIo("同步路径越界: $relativePath")
        return target
    }

    /**
     * 为冲突副本分配一个不冲突的本地路径（基于时间戳与序号）。
     */
    private fun allocateConflictBackup(source: Path): Path {
        val timestamp = DateTimeFormatter.ofPattern("yyyy-MM-dd HH-mm-ss")
            .withZone(ZoneId.systemDefault())
            .format(Instant.now())
        for (sequence in 0..ConflictResolver.MAX_SEQUENCE) {
            val name = ConflictResolver.copyName(
                source.fileName.toString(), ConflictResolver.ConflictSide.LOCAL, timestamp, sequence,
            )
            val candidate = source.resolveSibling(name)
            if (!Files.exists(candidate, LinkOption.NOFOLLOW_LINKS)) return candidate
        }
        throw AppError.LocalIo("无法分配冲突副本路径")
    }

    /**
     * 返回配置对应的挂载根路径；未配置或解析失败返回 null。
     */
    private fun mountIdentity(config: UserConfig): Path? {
        if (!config.mountConfigured || config.mountDir.isBlank()) return null
        return runCatching { JvmMountPaths.resolve(config.mountDir) }.getOrNull()
    }

    /**
     * 删除指定挂载根对应的云树 checkpoint 及其临时/备份文件。
     */
    private fun deleteMountCheckpoint(config: UserConfig) {
        val root = mountIdentity(config) ?: return
        val checkpoint = paths.cloudTreeCheckpoint(root)
        Files.deleteIfExists(checkpoint)
        Files.deleteIfExists(checkpoint.resolveSibling("${checkpoint.fileName}.tmp"))
        Files.deleteIfExists(checkpoint.resolveSibling("${checkpoint.fileName}.bak"))
    }

    /**
     * 阻塞式优雅关闭同步引擎。
     */
    override fun close() {
        kotlinx.coroutines.runBlocking { closeGracefully() }
    }

    /**
     * 标记关闭、停止同步源、等待活动周期结束，并在完成时删除未完成关机哨兵文件。
     */
    override suspend fun closeGracefully(timeoutMs: Long): Boolean {
        if (closed) return true
        closed = true
        started.set(false)
        stopSources()
        reconfigurationJob?.cancelAndJoin()
        Files.createDirectories(paths.dataDir)
        val sentinel = paths.dataDir.resolve("incomplete-shutdown")
        Files.writeString(sentinel, System.currentTimeMillis().toString())
        val completed = withTimeoutOrNull(timeoutMs) {
            activity.closeAndWait()
            true
        } ?: false
        scope.cancel("同步引擎关闭")
        if (completed) Files.deleteIfExists(sentinel)
        return completed
    }

    companion object {
        /**
         * 默认上传稳定性探针工厂：基于 JvmUploadStabilityProbe 对路径做稳定性检查。
         */
        private fun defaultStabilityProbe(): suspend (Path) -> UploadStability {
            val probe = JvmUploadStabilityProbe()
            return { path -> probe.check(path.toString()) }
        }
    }
}
