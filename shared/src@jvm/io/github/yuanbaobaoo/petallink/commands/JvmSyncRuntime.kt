package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.PlatformInode
import io.github.yuanbaobaoo.petallink.config.ConfigStore
import io.github.yuanbaobaoo.petallink.config.JvmMountPaths
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.core.logging.Logger
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
import io.github.yuanbaobaoo.petallink.sync.engine.ActionPlannerGuards
import io.github.yuanbaobaoo.petallink.sync.engine.ActivityTracker
import io.github.yuanbaobaoo.petallink.sync.engine.AntiOscillation
import io.github.yuanbaobaoo.petallink.sync.engine.BfsCloudTreeRefresher
import io.github.yuanbaobaoo.petallink.sync.engine.CloudTreeCache
import io.github.yuanbaobaoo.petallink.sync.engine.CycleCoordinator
import io.github.yuanbaobaoo.petallink.sync.engine.CycleRequest
import io.github.yuanbaobaoo.petallink.sync.engine.CycleRequestDispatcher
import io.github.yuanbaobaoo.petallink.sync.engine.DownloadTargetIdentity
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
import io.github.yuanbaobaoo.petallink.sync.identity.LocalMoveActionReconciler
import io.github.yuanbaobaoo.petallink.data.ColumnPatch
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.data.TransferPatch
import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.sync.executor.ActionResult
import io.github.yuanbaobaoo.petallink.sync.executor.SyncExecutor
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.NoSuchFileException
import java.nio.file.Path
import java.nio.file.attribute.BasicFileAttributes
import java.time.Instant
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
    private val ensureUploadCapacity: suspend (Long) -> Unit = {},
    private val isOnline: () -> Boolean = { true },
    private val onTransferChanged: suspend (Long) -> Unit = {},
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
    private val stabilityFactory: () -> (suspend (Path) -> UploadStability) = ::defaultStabilityProbe,
) : SyncCommandPlan {
    private val coordinator = CycleCoordinator()
    private val activity = ActivityTracker()
    private val dispatcher = CycleRequestDispatcher(scope, coordinator, ::runCycle)
    private val inodeMoveCoordinator = JvmInodeMoveCoordinator(db)
    private val antiOscillation = AntiOscillation()
    private val logger = Logger()
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
    private var recoveryJob: Job? = null
    private var cloudRefresherRoot: Path? = null
    private var cloudRefresher: BfsCloudTreeRefresher? = null
    private var cloudRefresherPatterns: List<String>? = null
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
                        cloudRefresherRoot = null
                        cloudRefresher = null
                        cloudRefresherPatterns = null
                    } else if (previous.skipPatterns != current.skipPatterns) {
                        // skipPatterns 变更：云树刷新器缓存随之失效，下轮按新模式过滤
                        cloudRefresherPatterns = null
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
        if (closed) {
            logger.info("sync.engine.lifecycle") { "引擎已 shutdown，跳过启动" }
            return
        }
        if (!started.compareAndSet(false, true)) return
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
                runCatching {
                    status.snapshot(db, RuntimeStatus(lastSyncTime = System.currentTimeMillis(), contentChanged = true))
                }.onFailure {
                    logger.warn("commands.folder_sync") { "目录同步完成后重算全局状态失败 error=${it.message}" }
                }
                logger.info("commands.folder_sync") {
                    "sync_folder_recursive（后台）完成 done=${mutableFolderSyncProgress.value?.done ?: 0} rel=$relativePath"
                }
                dispatcher.submit(CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_FULL)
            } catch (error: Throwable) {
                logger.warn("commands.folder_sync") { "sync_folder_recursive（后台）失败 error=${error.message} rel=$relativePath" }
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
        }.onFailure {
            logger.error("sync.engine.lifecycle", { "watcher启动失败: ${it.message}" }, it)
        }.getOrNull() ?: return@synchronized
        watcher = localWatcher
        watcherJob = scope.launch {
            localWatcher.changes.collect {
                if (started.get() && !closed && !reconfiguring.get()) dispatcher.submit(CycleRequest.LOCAL_RESCAN)
            }
        }
        runCatching(localWatcher::start).onFailure {
            logger.error("sync.engine.lifecycle", { "watcher启动失败: ${it.message}" }, it)
        }
        if (config.pollIntervalSec >= 60) {
            logger.info("sync.engine.lifecycle") { "启动云端定时刷新任务 interval_secs=${config.pollIntervalSec}" }
            timerJob = scope.launch {
                while (isActive && started.get() && !closed) {
                    delay(config.pollIntervalSec * 1_000)
                    if (started.get() && !closed && !reconfiguring.get()) {
                        dispatcher.submit(CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL)
                    }
                }
            }
        } else {
            logger.info("sync.engine.lifecycle") { "云端定时刷新已关闭（poll_interval_secs=0）" }
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
        recoveryJob?.cancel()
        recoveryJob = null
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
        val cycleStartedAtMs = System.currentTimeMillis()
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
        val refresher = cloudRefresher(root, store, config.skipPatterns)
        val cloud = try {
            val loaded = store.loadTrusted()
            when {
                request.contains(CycleRequest.CLOUD_FULL) -> refresher.refreshFull()
                request.contains(CycleRequest.CLOUD_INCREMENTAL) && loaded?.cursor != null ->
                    refresher.refreshIncremental(loaded.cursor)
                loaded != null -> loaded
                else -> refresher.refreshFull()
            }
        } catch (error: Throwable) {
            if (request.contains(CycleRequest.CLOUD_INCREMENTAL) && !request.contains(CycleRequest.CLOUD_FULL)) {
                if (request.contains(CycleRequest.STARTUP)) {
                    logger.warn("sync.engine.cycle") { "启动 owner 无法建立可信云端 checkpoint，禁止进入 planner error=${error.message}" }
                } else {
                    logger.warn("sync.engine.cycle") { "云端刷新失败，完整保留当前周期意图等待补跑 error=${error.message}" }
                }
            }
            throw error
        }
        runCatching { cloud.validateTrusted() }
            .onFailure {
                logger.warn("sync.engine.cycle") { "云端 checkpoint 尚未追平，跳过任务恢复与同步规划" }
            }
            .getOrThrow()
        purgeDeletedTombstones(cloud)

        // 路径恢复：云端改名/移动已核验的文件直接本地改名并重键基线（简化版 path_recovery，docs/06 §14.3）
        recoverCloudPathChanges(root, cloud)

        val recoveredFreeUp = JvmFreeUpService(
            root, paths, db, JvmPlaceholderManager(root, MacXattrAccess), FilesApiFreeUpVerifier(filesApi),
        ).recoverInterrupted()
        if (recoveredFreeUp > 0) {
            logger.warn("commands") { "启动前已收敛中断的释放空间操作 count=$recoveredFreeUp" }
        }

        val transferStore = JvmTransferFileStore()
        val stability = stabilityFactory()
        val transferPlaceholder = JvmPlaceholderManager(root, MacXattrAccess)
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
            ensureUploadCapacity = ensureUploadCapacity,
            downloadTargetProbe = transferPlaceholder::downloadTargetIdentity,
            backupModifiedPlaceholder = { transferPlaceholder.backupModifiedPlaceholder(it) },
        )
        val taskRunner = TaskRunner(
            db.transfers,
            transferOperations,
            isOnline,
            System::currentTimeMillis,
            maxConcurrentTransfers = config.concurrency,
            onTaskChanged = onTransferChanged,
        )
        if (request.contains(CycleRequest.STARTUP)) {
            runCatching { taskRunner.performStartupRecovery { resumePendingTransfers(taskRunner) } }
                .onFailure { logger.warn("sync.task_runner.recovery") { "启动任务恢复失败 error=${it.message}" } }
                .getOrThrow()
            // 启动时复位上次异常退出遗留的中间状态（对标 cycle.rs:241-246）
            db.syncItems.resetStaleStatuses()
        }
        if (request.contains(CycleRequest.RETRY)) {
            val ids = explicitRetries.toList()
            explicitRetries.removeAll(ids.toSet())
            if (ids.isEmpty()) {
                val retryable = db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.Failed) +
                    db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired)
                for (task in retryable) task.id?.let { retryTaskWithLog(taskRunner, it) }
            } else {
                for (id in ids) retryTaskWithLog(taskRunner, id)
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
        }.toMutableMap()
        // reconcile：本地无基线但 inode 映射有 fileId 的文件直接补建基线（简化版 reconcile_db_records，docs/06 §14.2）
        var reconciledCount = 0
        for (entry in localEntries) {
            if (entry.relativePath in baselines) continue
            val identity = db.inodeMap.lookup(entry.inode) ?: continue
            if (identity.relativePath != entry.relativePath) continue // 路径不同属移动检测范畴
            val cloudFile = cloud.tree[entry.relativePath]
            val cloudEditedMs = cloudFile?.editedTime?.let { raw ->
                runCatching { Instant.parse(raw).toEpochMilli() }.getOrNull()
            }
            db.syncItems.upsert(SyncItem(
                fileId = identity.fileId,
                localPath = entry.relativePath,
                parentFolderId = cloudFile?.singleParentOrNull,
                name = entry.relativePath.substringAfterLast('/'),
                isFolder = entry.isDirectory,
                size = cloudFile?.sizeBytes ?: 0L,
                localSize = if (entry.isDirectory) null else entry.size,
                sha256 = cloudFile?.contentHash,
                localMtime = entry.mtime,
                cloudEditedTime = cloudEditedMs,
                lastSyncTime = System.currentTimeMillis(),
                status = SyncStatus.SYNCED,
                errorMessage = null,
            ))
            baselines[entry.relativePath] = DbBaselineEntry(
                identity.fileId, entry.mtime, entry.size, cloudEditedMs, SyncStatus.SYNCED, entry.isDirectory,
            )
            reconciledCount++
        }
        if (reconciledCount > 0) {
            logger.info("sync.engine.reconciliation") { "已通过 inode 映射补建同步基线 count=$reconciledCount" }
        }
        val localInCloudNotDb = local.keys.filter { cloud.tree.containsKey(it) && !baselines.containsKey(it) }
        if (localInCloudNotDb.isNotEmpty()) {
            logger.debug("sync.engine.cycle") { "本地+云端有但DB无（reconcile 将补） count=${localInCloudNotDb.size} paths=$localInCloudNotDb" }
        }
        val inCloudDbNotLocal = cloud.tree.keys.filter { baselines.containsKey(it) && !local.containsKey(it) }
        if (inCloudDbNotLocal.isNotEmpty()) {
            logger.info("sync.engine.cycle") { "云端+DB有但本地无（应生成 DeleteFromCloud） count=${inCloudDbNotLocal.size} paths=$inCloudDbNotLocal" }
        }
        val snapshot = SyncSnapshot(local, cloud.tree, baselines, cloudTreeTrusted = true, isStartupResume = request.contains(CycleRequest.STARTUP))
        // 防振荡：本地删除后 TTL 内丢弃同路径回弹动作（保留 DeleteFromCloud，对标 cycle.rs:546）
        antiOscillation.purgeExpired(System.currentTimeMillis())
        val plannedActions = ActionPlannerGuards.prepare(
            snapshot,
            antiOscillation.filter(Planner.plan(snapshot)),
            recentlyDeleted = antiOscillation.paths(),
        )
        val detectedMoves = inodeMoveCoordinator.detect(localEntries, baselines, cloud)
        val actions = LocalMoveActionReconciler.reconcile(plannedActions, detectedMoves, cloud)
        if (actions.isEmpty()) {
            logger.info("sync.engine.cycle") {
                "sync cycle: 无操作，短路返回 triggered_by=${request.bits} local=${local.size} cloud=${cloud.tree.size} db=${baselines.size}"
            }
        } else {
            logger.info("sync.engine.cycle") { "sync cycle: 开始执行动作 triggered_by=${request.bits} actions=${actions.size}" }
        }
        val placeholder = JvmPlaceholderManager(root, MacXattrAccess)
        val executor = SyncExecutor(config.concurrency) { action ->
            executeAction(root, placeholder, taskRunner, transferStore, action)
        }
        val results = executor.executeActionsOrdered(actions, cloud.pathToId)

        // 远端写先合并进同一 checkpoint，再提交 DB 基线。
        val committedCloud = commitRemoteWrites(store, cloud, actions, results)
        settleBaselines(root, committedCloud, actions, results)
        // G 分支：双方都无的 DB 残余在周期末尾清理（对标 Planner.kt G 注释与 cycle 末尾残余清理）
        var residualCount = 0
        for (item in db.syncItems.selectAll()) {
            if (item.localPath in local || item.localPath in committedCloud.tree) continue
            if (item.status == SyncStatus.DELETED) continue // 墓碑由 purgeDeletedTombstones 统一处理
            db.syncItems.deleteByFileId(item.fileId)
            residualCount++
        }
        if (residualCount > 0) {
            logger.info("sync.engine.reconciliation") { "已清理双方都无的 DB 残余 count=$residualCount" }
        }
        inodeMoveCoordinator.refresh(localEntries, freshSinceMs = cycleStartedAtMs)
        if (actions.zip(results).any { (action, result) -> result.success && action.type == SyncActionType.MOVE_IN_CLOUD }) {
            // 移动只结算结构事实并保留内容基线，立即重扫以版本校验方式上传并发编辑（对标 cycle.rs:623-628）
            dispatcher.submit(CycleRequest.LOCAL_RESCAN)
        }
        val failed = results.withIndex().filter { !it.value.success && !it.value.deferred }
        status.snapshot(db, RuntimeStatus(lastSyncTime = System.currentTimeMillis(), contentChanged = actions.isNotEmpty()))
            if (failed.isEmpty() && actions.isNotEmpty()) {
                val contentChanged = actions.zip(results).any { (action, result) ->
                    result.success && action.type != SyncActionType.SKIP
                }
                logger.info("sync.engine.cycle") {
                    "sync cycle ok triggered_by=${request.bits} actions=${actions.size} content_changed=$contentChanged"
                }
            }
            if (failed.isNotEmpty()) throw AppError.Data("同步周期有 ${failed.size} 个动作失败")
        }
        if (result.isFailure && !closed) {
            logger.warn("sync.engine.cycle") { "后台协调周期失败 error=${result.exceptionOrNull()?.message}" }
            runCatching {
                status.snapshot(db, RuntimeStatus(lastSyncTime = System.currentTimeMillis()))
            }
        }
        runCatching { scheduleRecoveryDeadline() }
            .onFailure { logger.warn("sync.engine.lifecycle") { "退避 deadline 恢复周期失败 error=${it.message}" } }
        return result
        } finally {
            guard.close()
        }
    }

    /**
     * 为最近的退避、远端核验或重规划截止时间安排一次恢复周期，不在计时器中直接执行传输。
     * RestartRequired 没有持久 deadline：由下一轮 planner intent 自动重规划，这里给一个有界
     * 延迟投递恢复周期，避免编辑中的文件停手后停滞（同时防止无间隔热循环）。
     */
    private suspend fun scheduleRecoveryDeadline() {
        recoveryJob?.cancel()
        if (closed || !started.get() || reconfiguring.get()) return
        val now = System.currentTimeMillis()
        val candidates = runCatching {
            buildList {
                addAll(db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.BackingOff).map { it.nextRetryAt })
                addAll(db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.VerifyingRemote).map { it.nextRetryAt })
                addAll(
                    db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired)
                        .map { it.nextRetryAt ?: now + RESTART_REQUIRED_RECOVERY_DELAY_MS },
                )
            }
        }.onFailure {
            logger.warn("sync.engine.lifecycle") { "读取退避 deadline 失败 error=${it.message}" }
        }.getOrNull() ?: return
        val deadline = candidates.filterNotNull().minOrNull() ?: return
        recoveryJob = scope.launch {
            delay((deadline - System.currentTimeMillis()).coerceAtLeast(0L))
            if (started.get() && !closed && !reconfiguring.get()) {
                dispatcher.submit(CycleRequest.ONLINE_RECOVERY)
            }
        }
    }

    private fun cloudRefresher(root: Path, store: JvmCloudTreeCheckpointStore, skipPatterns: List<String>): BfsCloudTreeRefresher {
        val normalized = root.toAbsolutePath().normalize()
        val existing = cloudRefresher
        if (cloudRefresherRoot == normalized && existing != null && cloudRefresherPatterns == skipPatterns) return existing
        return BfsCloudTreeRefresher(filesApi, changesApi, store, skipPatterns = skipPatterns).also {
            cloudRefresherRoot = normalized
            cloudRefresher = it
            cloudRefresherPatterns = skipPatterns
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
        logger.debug("sync.executor.actions") { "executor: 开始执行 rel=${action.relativePath} action_type=${action.type}" }
        val conflict = JvmConflictCoordinator(
            root,
            placeholder,
            executeTransfer = { transferAction ->
                executeTransferAction(root, placeholder, taskRunner, transferStore, transferAction)
            },
            hasActiveUpload = { relativePath ->
                activeTransfer(relativePath, TransferDirection.UPLOAD) != null
            },
        )
        val result = try {
            when (action.type) {
            SyncActionType.CREATE_PLACEHOLDER -> {
                // 目标已存在用户内容时拒绝建占位符，避免把内容分歧误结算为已同步（对标 manager.rs:184-204）
                when (placeholder.downloadTargetIdentity(safeLocalPath(root, action.relativePath).toString())) {
                    is DownloadTargetIdentity.Occupied, DownloadTargetIdentity.Inaccessible ->
                        throw AppError.Conflict("占位符创建被拒绝：目标已存在本地内容: ${action.relativePath}")
                    else -> {
                        placeholder.createPlaceholderIfNeeded(action.relativePath)
                        ActionResult(true)
                    }
                }
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
                conflict.backupBeforeCloudDelete(action.relativePath)
                ActionResult(true)
            }
            SyncActionType.SKIP -> ActionResult(true)
            SyncActionType.DOWNLOAD -> executeTransferAction(root, placeholder, taskRunner, transferStore, action)
            SyncActionType.DELETE_FROM_CLOUD -> deleteFromCloudGuarded(root, placeholder, action)
            SyncActionType.DELETE_FROM_LOCAL -> deleteFromLocalGuarded(root, placeholder, action).also { result ->
                // 记录最近删除路径，TTL 内抑制云树回弹重建（对标 results.rs:381-437）
                if (result.success) antiOscillation.addDeleted(action.relativePath, System.currentTimeMillis())
            }
            SyncActionType.MOVE_IN_CLOUD -> moveInCloudGuarded(root, action)
            SyncActionType.CREATE_CONFLICT_COPY -> {
                conflict.execute(action)
            }
            }
        } catch (error: Throwable) {
            if (action.type == SyncActionType.UPLOAD) {
                mutableUploadFailures.emit(UploadFailedEvent(action.relativePath, error.message ?: "上传失败"))
            }
            ActionResult(false, errorMessage = error.message)
        }
        logActionResult(action, result)
        return result
    }

    /**
     * 统一记录动作执行结果日志（对标 executor/actions.rs log_action_result；deferred 时不打失败日志）。
     */
    private fun logActionResult(action: SyncAction, result: ActionResult) {
        val (verbSuccess, verbFail) = when (action.type) {
            SyncActionType.CREATE_FOLDER -> "创建目录成功" to "创建目录失败"
            SyncActionType.MOVE_IN_CLOUD -> "更新云端文件路径成功" to "更新云端文件路径失败"
            SyncActionType.DELETE_FROM_CLOUD -> "删除云端文件成功" to "删除云端文件失败"
            SyncActionType.DELETE_FROM_LOCAL -> "删除本地文件成功" to "删除本地文件失败"
            SyncActionType.CREATE_CONFLICT_COPY -> "冲突处理完成" to "冲突处理失败"
            else -> return
        }
        if (result.success) {
            logger.info("sync.executor.actions") { "$verbSuccess rel=${action.relativePath}" }
        } else if (!result.deferred) {
            logger.warn("sync.executor.actions") { "$verbFail rel=${action.relativePath} error=${result.errorMessage}" }
        }
    }

    /**
     * 取消破坏性动作并返回延期结果（不推进基线、不计周期失败，等待下一轮重新规划）。
     */
    private fun deferredCancel(action: SyncAction, message: String): ActionResult {
        logger.warn("sync.executor.actions") { "已取消破坏性动作 rel=${action.relativePath} action_type=${action.type} reason=$message" }
        return ActionResult(false, deferred = true, errorMessage = message)
    }

    /**
     * 读取路径是否存在；读取异常返回 null（无法证明缺失）。
     */
    private fun localPathExists(path: Path): Boolean? = try {
        Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
        true
    } catch (_: NoSuchFileException) {
        false
    } catch (_: Throwable) {
        null
    }

    /**
     * 仅在本地内容仍匹配持久基线且云端删除事实已确认时执行本地删除
     * （对标 local_delete.rs:101-231 do_delete_from_local：先核对基线快照，verify_deleted 后复核）。
     */
    private suspend fun deleteFromLocalGuarded(
        root: Path,
        placeholder: JvmPlaceholderManager,
        action: SyncAction,
    ): ActionResult {
        val items = try {
            db.syncItems.selectAll()
        } catch (error: Throwable) {
            return deferredCancel(action, "读取同步基线失败，保留本地内容: ${error.message}")
        }
        val baselines = HashMap<String, SyncItem>(items.size)
        for (item in items) {
            if (baselines.put(item.localPath, item) != null) {
                return deferredCancel(action, "同步基线存在重复路径，拒绝删除: ${item.localPath}")
            }
        }
        val path = safeLocalPath(root, action.relativePath)
        val allowOrphanPlaceholder = action.fileId == null
        var pathExists = localPathExists(path)
            ?: return deferredCancel(action, "无法读取待删除路径，保留本地内容")
        if (pathExists) {
            try {
                verifyLocalDeleteSnapshot(path, action.relativePath, placeholder, baselines, allowOrphanPlaceholder)
            } catch (error: Throwable) {
                return deferredCancel(action, error.message ?: "本地快照核验失败")
            }
        }
        // 远端删除证明尽量贴近不可逆的本地删除动作
        val fileId = action.fileId
        if (fileId != null) {
            if (fileId.startsWith(PENDING_FILE_ID_PREFIX)) {
                return deferredCancel(action, "待上传记录没有可核验的远端删除事实")
            }
            val deleted = try {
                filesApi.verifyDeleted(fileId)
            } catch (error: Throwable) {
                return deferredCancel(action, "无法确认云端已删除，保留本地内容: ${error.message}")
            }
            if (!deleted) return deferredCancel(action, "云端文件仍存在，取消本地删除并等待重新规划")
        }
        // 远端核验返回后重新检查完整本地快照
        if (pathExists) {
            pathExists = localPathExists(path)
                ?: return deferredCancel(action, "远端核验后无法读取待删除路径，保留本地内容")
            if (pathExists) {
                try {
                    verifyLocalDeleteSnapshot(path, action.relativePath, placeholder, baselines, allowOrphanPlaceholder)
                } catch (error: Throwable) {
                    return deferredCancel(action, "远端核验期间本地内容发生变化，已取消删除: ${error.message}")
                }
            }
        }
        if (!pathExists) return ActionResult(true)
        return try {
            placeholder.deleteLocal(path.toString())
            ActionResult(true)
        } catch (error: Throwable) {
            deferredCancel(action, error.message ?: "删除本地内容失败")
        }
    }

    /**
     * 核验待删除内容仍与持久化同步基线一致（对标 local_delete.rs:22-99 verify_local_delete_snapshot）。
     * 目录只核对身份并递归核验子项：CMP 目录基线 mtime 会被后续占位符/下载落地刷新而产生
     * 无害偏差（原项目靠 reconcile 每轮刷新目录基线），严格比对会永久卡住目录删除。
     */
    private suspend fun verifyLocalDeleteSnapshot(
        path: Path,
        relativePath: String,
        placeholder: JvmPlaceholderManager,
        baselines: Map<String, SyncItem>,
        allowOrphanPlaceholder: Boolean,
    ) {
        if (Files.isSymbolicLink(path)) throw AppError.LocalIo("待删除路径已变为符号链接: $relativePath")
        if (Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS)) {
            val baseline = baselines[relativePath]
                ?: throw AppError.Data("目录不在同步基线中: $relativePath")
            if (!baseline.isFolder) throw AppError.Data("目录类型与同步基线不一致: $relativePath")
            Files.newDirectoryStream(path).use { children ->
                for (child in children) {
                    val name = child.fileName.toString()
                    val childRelative = if (relativePath.isEmpty()) name else "$relativePath/$name"
                    verifyLocalDeleteSnapshot(child, childRelative, placeholder, baselines, false)
                }
            }
            return
        }
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("拒绝删除非普通文件: $relativePath")
        }
        if (placeholder.isPlaceholder(path.toString())) {
            if (allowOrphanPlaceholder) return
            val baseline = baselines[relativePath]
                ?: throw AppError.Data("占位符不在同步基线中: $relativePath")
            if (baseline.isFolder) throw AppError.Data("占位符类型与同步基线不一致: $relativePath")
            return
        }
        val baseline = baselines[relativePath]
            ?: throw AppError.Data("文件不在同步基线中: $relativePath")
        if (baseline.isFolder ||
            baseline.localMtime != Files.getLastModifiedTime(path, LinkOption.NOFOLLOW_LINKS).toMillis() ||
            baseline.localSize != Files.size(path)
        ) {
            throw AppError.Data("文件在删除执行前发生变化: $relativePath")
        }
    }

    /**
     * 云端删除前复核本地路径：文件实际存在（非占位符）或无法证明缺失时降级，拒绝删除
     * （对标 reconciliation.rs:712-758 validate_delete_from_cloud）。
     */
    private suspend fun deleteFromCloudGuarded(
        root: Path,
        placeholder: JvmPlaceholderManager,
        action: SyncAction,
    ): ActionResult {
        val id = action.fileId ?: throw AppError.Data("云端删除缺少 fileId")
        val local = safeLocalPath(root, action.relativePath)
        val exists = localPathExists(local)
            ?: return deferredCancel(action, "本地路径访问异常，无法证明文件已删除")
        if (exists && !placeholder.isPlaceholder(local.toString())) {
            val size = runCatching { Files.size(local) }.getOrNull()
            return deferredCancel(action, "防误删：本地文件实际存在（${size ?: "?"} 字节），跳过 DeleteFromCloud")
        }
        filesApi.deleteFile(id)
        return ActionResult(true)
    }

    /**
     * 云端移动执行前两道校验：本地身份复核（inode 映射仍属同一 fileId，对标 actions.rs:344-358）
     * 与目标目录同名预检（对标 actions.rs:365-378），任一不符则取消并等待重新规划。
     */
    private suspend fun moveInCloudGuarded(root: Path, action: SyncAction): ActionResult {
        val id = action.fileId ?: throw AppError.Data("云端移动缺少 fileId")
        val localPath = safeLocalPath(root, action.relativePath)
        val localIdentity = runCatching {
            db.inodeMap.lookup(PlatformInode.readInode(localPath.toString()))?.fileId
        }.getOrNull()
        if (localIdentity != id) {
            return deferredCancel(action, "云端路径变更执行前本地路径或 fileId 已变化，等待重新规划")
        }
        val desiredName = action.relativePath.substringAfterLast('/')
        val desiredParent = action.parentFileId
        if (desiredParent.isNullOrBlank()) {
            return deferredCancel(action, "云端路径变更的目标父目录尚未取得 fileId，等待重新规划")
        }
        val siblings = try {
            filesApi.listAllFiles(desiredParent)
        } catch (error: Throwable) {
            return deferredCancel(action, "核验移动目标目录失败，未发送远端写入: ${error.message}")
        }
        if (siblings.any { it.id != null && it.id != id && it.name == desiredName }) {
            return deferredCancel(action, "目标目录已存在同名云端文件 $desiredName，拒绝覆盖并等待重新规划")
        }
        var remote = filesApi.getFile(id)
        if (remote.name != desiredName) remote = filesApi.updateFile(id, desiredName)
        val currentParent = runCatching { io.github.yuanbaobaoo.petallink.drive.DriveParsers.singleParent(remote) }.getOrNull()
        if (currentParent != desiredParent) {
            if (currentParent.isNullOrBlank()) throw AppError.Data("云端移动缺少旧 parent")
            remote = filesApi.moveFile(id, currentParent, desiredParent)
        }
        return ActionResult(true, cloudFileId = id, cloudFile = remote)
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
        var active = activeTransfer(action.relativePath, direction)
        val restartCandidate = active
        if (restartCandidate?.state == io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired) {
            // 新一轮 planner intent 到达：自动重规划 RestartRequired 任务（对标 admission.rs:153-174,292-414）
            val promoted = try {
                restartOrPromote(restartCandidate)
            } catch (error: Throwable) {
                logger.warn("sync.engine.cycle") {
                    "RestartRequired 自动重规划冲突，等待下轮恢复 task_id=${restartCandidate.id} rel=${action.relativePath} error=${error.message}"
                }
                return ActionResult(false, deferred = true, errorMessage = error.message ?: "RESTART_REQUIRED")
            }
            if (promoted != null) {
                return ActionResult(false, deferred = true, errorMessage = promoted.errorMessage ?: TaskDisposition.VERIFYING_REMOTE.name)
            }
            active = null
        }
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
        if (current.state == io.github.yuanbaobaoo.petallink.sync.TransferState.Failed) {
            // Failed 是自动任务的终态：任务行与可见错误作为路径屏障保留，等显式重试（对标 admission.rs:175-181）
            return ActionResult(
                false,
                deferred = true,
                errorMessage = current.errorMessage ?: TaskDisposition.FAILED.name,
            )
        }
        val disposition = when (current.state) {
            io.github.yuanbaobaoo.petallink.sync.TransferState.Pending,
            io.github.yuanbaobaoo.petallink.sync.TransferState.WaitingForNetwork,
            io.github.yuanbaobaoo.petallink.sync.TransferState.BackingOff -> taskRunner.runExpected(current.toTaskContext())
            io.github.yuanbaobaoo.petallink.sync.TransferState.Completed -> TaskDisposition.COMPLETED
            io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired -> TaskDisposition.RESTART_REQUIRED
            io.github.yuanbaobaoo.petallink.sync.TransferState.VerifyingRemote -> TaskDisposition.VERIFYING_REMOTE
            io.github.yuanbaobaoo.petallink.sync.TransferState.Running -> TaskDisposition.BLOCKED
            io.github.yuanbaobaoo.petallink.sync.TransferState.Canceled -> TaskDisposition.CANCELED
            io.github.yuanbaobaoo.petallink.sync.TransferState.Failed -> TaskDisposition.FAILED
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
            // 下载完成后即时记账 inode 映射（对标 transfer_operations.rs:380-381 的 set_file_id_xattr；
            // xattr 随 inode 走，下轮刷新前改名也不丢身份）
            val downloadedFileId = action.fileId
            if (downloadedFileId != null) {
                runCatching {
                    db.inodeMap.upsert(
                        PlatformInode.readInode(destination.toString()),
                        action.relativePath,
                        downloadedFileId,
                        System.currentTimeMillis(),
                    )
                }.onFailure {
                    logger.warn("sync.engine.reconciliation") { "下载完成后更新 inode 映射失败 rel=${action.relativePath} error=${it.message}" }
                }
            }
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
     * RestartRequired 任务的自动恢复（对标 admission.rs:153-158 promote_restart_to_verifying 与
     * admission.rs:292-414 replan_task）：含持久化远端结果的任务禁止重放，提升为待核验并返回；
     * 其余取消被取代的旧任务并返回 null，由调用方以全新 planner intent 重新入队（源快照随新意图刷新）。
     */
    private suspend fun restartOrPromote(task: TransferTask): TransferTask? {
        val taskId = task.id ?: throw AppError.Data("重规划任务缺少 id")
        if (!task.remoteResultFileId.isNullOrBlank()) {
            val promoted = db.transfers.transition(
                taskId,
                task.stateRevision,
                io.github.yuanbaobaoo.petallink.sync.TransferState.VerifyingRemote,
                TransferPatch(
                    errorMessage = ColumnPatch.Set("远端写入已返回资源 ID，禁止重放并等待核验"),
                    nextRetryAt = ColumnPatch.Set(System.currentTimeMillis()),
                    finishedAt = ColumnPatch.Clear,
                ),
            )
            logger.info("sync.engine.cycle") { "含远端结果的重规划任务已恢复为核验态 task_id=$taskId rel=${task.relativePath}" }
            return promoted
        }
        db.transfers.transition(
            taskId,
            task.stateRevision,
            io.github.yuanbaobaoo.petallink.sync.TransferState.Canceled,
            TransferPatch(
                errorMessage = ColumnPatch.Set("新的 planner intent 已取代尚未执行的旧任务"),
                finishedAt = ColumnPatch.Set(System.currentTimeMillis()),
            ),
        )
        logger.info("sync.engine.cycle") { "RestartRequired 任务已按新 planner intent 自动重规划 task_id=$taskId rel=${task.relativePath}" }
        return null
    }

    /**
     * 选定云端目录的后台 BFS 双向同步；文件下载为真实内容而非占位符。
     */
    private suspend fun syncFolderSubtree(folderId: String, relativePath: String) {
        require(folderId.isNotBlank()) { "目录 fileId 不能为空" }
        logger.info("commands.folder_sync") { "sync_folder_recursive: 开始递归同步 folder_id=$folderId rel=$relativePath" }
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
        logger.info("commands.folder_sync") { "sync_folder_recursive: 云端子树 files=${cloudFiles.size} folders=${cloudFolders.size}" }

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
            logger.info("commands.folder_sync") { "sync_folder_recursive: 已补建云端父目录 dir=${entry.relativePath} cloud_id=${remote.id}" }
        }

        val transferStore = JvmTransferFileStore()
        val stability = stabilityFactory()
        val transferPlaceholder = JvmPlaceholderManager(root, MacXattrAccess)
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
            ensureUploadCapacity = ensureUploadCapacity,
            downloadTargetProbe = transferPlaceholder::downloadTargetIdentity,
            backupModifiedPlaceholder = { transferPlaceholder.backupModifiedPlaceholder(it) },
        )
        val runner = TaskRunner(
            db.transfers,
            operations,
            isOnline,
            System::currentTimeMillis,
            maxConcurrentTransfers = config.concurrency,
        )
        val placeholder = JvmPlaceholderManager(root, MacXattrAccess)
        val actions = mutableListOf<SyncAction>()
        val results = mutableListOf<ActionResult>()

        val downloads = cloudFiles.filter { (subPath, _) ->
            val local = localBySubPath[subPath]
            local == null || local.isPlaceholder
        }
        val uploads = localEntries.filter { !it.isDirectory && !it.isPlaceholder && it.relativePath !in cloudFiles }
        logger.info("commands.folder_sync") { "sync_folder_recursive: 任务 download=${downloads.size} upload=${uploads.size}" }
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
            val result = executeTransferAction(root, placeholder, runner, transferStore, action)
            results += result
            if (result.deferred) {
                logger.warn("commands.folder_sync") { "sync_folder_recursive: 下载进入恢复队列 subrel=$subPath disposition=${result.errorMessage}" }
            } else if (!result.success) {
                logger.warn("commands.folder_sync") { "sync_folder_recursive: 下载失败 subrel=$subPath error=${result.errorMessage}" }
            }
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
            val result = executeTransferAction(root, placeholder, runner, transferStore, action)
            results += result
            if (result.deferred) {
                logger.warn("commands.folder_sync") { "sync_folder_recursive: 上传进入恢复队列 subrel=${local.relativePath} disposition=${result.errorMessage}" }
            } else if (!result.success) {
                logger.warn("commands.folder_sync") { "sync_folder_recursive: 上传失败 subrel=${local.relativePath} error=${result.errorMessage}" }
            }
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
        logger.info("commands.folder_sync") { "sync_folder_recursive: 完成 done=$done total=$total" }
    }

    private fun joinRelative(base: String, child: String): String = when {
        base.isBlank() -> child
        child.isBlank() -> base
        else -> "${base.trimEnd('/')}/$child"
    }

    private suspend fun resumePendingTransfers(taskRunner: TaskRunner) {
        for (task in db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.Pending)) {
            taskRunner.runExpected(task.toTaskContext())
        }
    }

    /**
     * 显式重试单个失败任务并记录结果日志（对标 cycle.rs 全局重试的两处 warn）。
     */
    private suspend fun retryTaskWithLog(taskRunner: TaskRunner, taskId: Long) {
        val disposition = runCatching { taskRunner.retryExplicit(taskId) }
            .onFailure {
                logger.warn("sync.engine.cycle") { "全局重试执行失败，状态已由任务机保留 task_id=$taskId error=${it.message}" }
            }
            .getOrThrow()
        if (disposition == TaskDisposition.BLOCKED) {
            logger.warn("sync.engine.cycle") { "失败任务未通过重试前置校验 task_id=$taskId error=$disposition" }
        }
    }

    private suspend fun activeTransfer(relativePath: String, direction: TransferDirection): TransferTask? {
        val states = listOf(
            io.github.yuanbaobaoo.petallink.sync.TransferState.Pending,
            io.github.yuanbaobaoo.petallink.sync.TransferState.Running,
            io.github.yuanbaobaoo.petallink.sync.TransferState.WaitingForNetwork,
            io.github.yuanbaobaoo.petallink.sync.TransferState.BackingOff,
            io.github.yuanbaobaoo.petallink.sync.TransferState.VerifyingRemote,
            io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired,
            // Failed 也视为路径占用：屏障期内复用既有失败任务，避免每周期新建任务热循环
            io.github.yuanbaobaoo.petallink.sync.TransferState.Failed,
        )
        val matches = mutableListOf<TransferTask>()
        for (state in states) {
            matches += db.transfers.selectByState(state).filter {
                it.relativePath == relativePath && it.direction == direction
            }
        }
        return matches.maxWithOrNull(compareBy<TransferTask> { it.createdAt }.thenBy { it.id ?: 0L })
    }

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
        logger.info("sync.engine.cache") { "已将恢复任务的权威远端结果提交到云端检查点 recovered=${actions.size}" }
        return candidate
    }

    /**
     * 根据动作执行结果更新 DB 同步基线：删除项移除，成功项 upsert，非延期失败落 FAILED
     * （对标 results.rs:69-130、182-186）。
     */
    private suspend fun settleBaselines(
        root: Path,
        cloud: CloudTreeCache,
        actions: List<SyncAction>,
        results: List<ActionResult>,
    ) = db.withTransaction {
        actions.zip(results).forEach { (action, result) ->
            if (!result.success) {
                // 失败或延期的动作不得推进基线；非延期失败只更新兼容状态与消息（TaskRunner 是权威失败来源）
                if (!result.deferred) {
                    val failedFileId = action.fileId?.takeUnless { it.startsWith(PENDING_FILE_ID_PREFIX) }
                    if (failedFileId != null) {
                        runCatching {
                            db.syncItems.updateStatus(failedFileId, action.relativePath, SyncStatus.FAILED, result.errorMessage)
                        }.onFailure {
                            logger.warn("sync.engine.cycle") { "记录同步失败状态失败 rel=${action.relativePath} error=${it.message}" }
                        }
                    }
                }
                return@forEach
            }
            if (action.type == SyncActionType.DELETE_FROM_CLOUD || action.type == SyncActionType.DELETE_FROM_LOCAL) {
                action.fileId?.let { db.syncItems.deleteByFileId(it) }
                    ?: db.syncItems.deleteByLocalPath(action.relativePath)
                return@forEach
            }
            if (action.type == SyncActionType.MOVE_IN_CLOUD) {
                inodeMoveCoordinator.settleSubtree(cloud, action)
                return@forEach
            }
            val remote: DriveFile = result.cloudFile ?: action.cloudFile ?: cloud.tree[action.relativePath] ?: return@forEach
            val fileId = remote.id ?: return@forEach
            val localPath = safeLocalPath(root, action.relativePath)
            val exists = Files.exists(localPath, LinkOption.NOFOLLOW_LINKS)
            // 上传结算记录实际送达的源快照（持久化在任务里的 sourceMtime/sourceSize），而非结算时刻的
            // 当前 stat（对标 settlement.rs:240-251）；上传期间的编辑由下一轮 planner 发现差异补传。
            val uploadSnapshot = if (action.type == SyncActionType.UPLOAD) {
                completedUploadSnapshot(action.relativePath)
                    ?: run {
                        logger.warn("sync.engine.cycle") { "上传成功结算缺少源快照，跳过基线写入 rel=${action.relativePath}" }
                        return@forEach
                    }
            } else null
            db.syncItems.upsert(SyncItem(
                fileId = fileId,
                localPath = action.relativePath,
                parentFolderId = remote.singleParentOrNull,
                name = remote.name ?: action.relativePath.substringAfterLast('/'),
                isFolder = Files.isDirectory(localPath, LinkOption.NOFOLLOW_LINKS),
                size = remote.sizeBytes,
                localSize = uploadSnapshot?.second
                    ?: if (exists && Files.isRegularFile(localPath, LinkOption.NOFOLLOW_LINKS)) Files.size(localPath) else null,
                sha256 = remote.contentHash,
                localMtime = uploadSnapshot?.first
                    ?: if (exists) Files.getLastModifiedTime(localPath, LinkOption.NOFOLLOW_LINKS).toMillis() else null,
                cloudEditedTime = remote.editedTime?.let { runCatching { Instant.parse(it).toEpochMilli() }.getOrNull() },
                lastSyncTime = System.currentTimeMillis(),
                status = if (action.type == SyncActionType.CREATE_CONFLICT_COPY) SyncStatus.CONFLICT else SyncStatus.SYNCED,
                errorMessage = null,
            ))
        }
    }

    /**
     * 读取该路径最近完成的持久化上传任务的源快照（mtime/size），缺失返回 null。
     */
    private suspend fun completedUploadSnapshot(relativePath: String): Pair<Long, Long>? {
        val task = db.transfers.selectByState(io.github.yuanbaobaoo.petallink.sync.TransferState.Completed)
            .filter { it.relativePath == relativePath && it.direction == TransferDirection.UPLOAD }
            .maxWithOrNull(compareBy<TransferTask> { it.createdAt }.thenBy { it.id ?: 0L })
        val mtime = task?.sourceMtime
        val size = task?.sourceSize
        return if (mtime != null && size != null) mtime to size else null
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

    private suspend fun purgeDeletedTombstones(cloud: CloudTreeCache) {
        val liveIds = cloud.tree.values.mapNotNull(DriveFile::id).toHashSet()
        var purged = 0
        for (item in db.syncItems.selectByStatus(SyncStatus.DELETED)) {
            if (item.fileId !in liveIds) {
                db.syncItems.deleteByFileId(item.fileId)
                purged++
            }
        }
        if (purged > 0) logger.info("sync.engine.cache") { "已清理可信云树中不存在的墓碑 count=$purged" }
    }

    /**
     * 简化版 path_recovery（docs/06 §14.3）：云端改名/移动后，本地文件直接改名并重键基线，
     * 避免"删本地 + 重建占位符"丢弃已下载内容。
     *
     * 安全闸（inode 方案下已大幅简化）：仅处理 SYNCED 文件基线；本地内容必须与基线一致
     * （不一致说明有未上传修改，走备份副本的常规流程）；新路径未被占用；目录交由常规
     * 删除+重建路径处理。任一不符跳过并保留给 planner 正常裁决。
     */
    private suspend fun recoverCloudPathChanges(root: Path, cloud: CloudTreeCache) {
        val cloudPathById = mutableMapOf<String, String>()
        for ((path, file) in cloud.tree) file.id?.let { cloudPathById[it] = path }
        var recovered = 0
        for (item in db.syncItems.selectAll()) {
            if (item.isFolder || item.status != SyncStatus.SYNCED) continue
            if (cloud.tree.containsKey(item.localPath)) continue // 路径未变
            val newPath = cloudPathById[item.fileId] ?: continue // 同 fileId 不在云端他处 → 真删除
            try {
                val oldLocal = safeLocalPath(root, item.localPath)
                val newLocal = safeLocalPath(root, newPath)
                if (!Files.exists(oldLocal, LinkOption.NOFOLLOW_LINKS)) continue
                if (Files.exists(newLocal, LinkOption.NOFOLLOW_LINKS)) {
                    logger.warn("sync.engine.reconciliation") { "路径恢复跳过：新路径已被占用 old=${item.localPath} new=$newPath file_id=${item.fileId}" }
                    continue
                }
                val currentMtime = Files.getLastModifiedTime(oldLocal, LinkOption.NOFOLLOW_LINKS).toMillis()
                val currentSize = if (Files.isRegularFile(oldLocal, LinkOption.NOFOLLOW_LINKS)) Files.size(oldLocal) else null
                if (item.localMtime != currentMtime || (currentSize != null && item.localSize != currentSize)) {
                    logger.warn("sync.engine.reconciliation") { "路径恢复跳过：本地内容与基线不一致 old=${item.localPath} file_id=${item.fileId}" }
                    continue
                }
                newLocal.parent?.let(Files::createDirectories)
                Files.move(oldLocal, newLocal)
                val remote = cloud.tree[newPath]
                db.syncItems.upsert(item.copy(
                    localPath = newPath,
                    parentFolderId = remote?.singleParentOrNull,
                    name = remote?.name ?: newPath.substringAfterLast('/'),
                    size = remote?.sizeBytes ?: item.size,
                    cloudEditedTime = remote?.editedTime
                        ?.let { runCatching { Instant.parse(it).toEpochMilli() }.getOrNull() } ?: item.cloudEditedTime,
                ))
                runCatching {
                    db.inodeMap.upsert(
                        PlatformInode.readInode(newLocal.toString()),
                        newPath, item.fileId, System.currentTimeMillis(),
                    )
                }
                recovered++
                logger.info("sync.engine.reconciliation") { "已收敛中断的远端路径变更 old=${item.localPath} new=$newPath file_id=${item.fileId}" }
            } catch (error: Throwable) {
                logger.warn("sync.engine.reconciliation") { "单个远端路径变化暂不能安全恢复，本轮仅隔离当前身份 old=${item.localPath} new=$newPath file_id=${item.fileId} error=${error.message}" }
            }
        }
        if (recovered > 0) logger.info("sync.engine.reconciliation") { "已在同步规划前收敛中断的远端路径变更 recovered=$recovered" }
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
        logger.info("sync.engine.lifecycle") { "SyncEngine shutdown_sync（shutdown 标志置位、FSEvents 释放）" }
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
         * pending: 上传待确认基线的 fileId 前缀（对标 PENDING_FILE_ID_PREFIX）。
         */
        private const val PENDING_FILE_ID_PREFIX = "pending:"

        /**
         * RestartRequired 任务的恢复周期延迟：由下一轮 planner intent 自动重规划，延迟避免热循环。
         */
        private const val RESTART_REQUIRED_RECOVERY_DELAY_MS = 5_000L

        /**
         * 默认上传稳定性探针工厂：基于 JvmUploadStabilityProbe 对路径做稳定性检查。
         */
        private fun defaultStabilityProbe(): suspend (Path) -> UploadStability {
            val probe = JvmUploadStabilityProbe()
            return { path -> probe.check(path.toString()) }
        }
    }
}
