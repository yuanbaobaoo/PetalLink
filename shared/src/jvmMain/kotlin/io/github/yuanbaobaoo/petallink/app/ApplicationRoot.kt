package io.github.yuanbaobaoo.petallink.app

import io.github.yuanbaobaoo.petallink.commands.AppResult
import io.github.yuanbaobaoo.petallink.commands.CommandService
import io.github.yuanbaobaoo.petallink.config.ConfigValidator
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.core.logging.Logger
import io.github.yuanbaobaoo.petallink.core.net_guard.NetGuard
import io.github.yuanbaobaoo.petallink.core.net_guard.NetState
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.drive.totalBytes
import io.github.yuanbaobaoo.petallink.drive.usedBytes
import io.github.yuanbaobaoo.petallink.sync.isFolder
import io.github.yuanbaobaoo.petallink.ui.pages.main.LogRecordDisplay
import io.github.yuanbaobaoo.petallink.ui.viewmodel.BrowserBreadcrumb
import io.github.yuanbaobaoo.petallink.ui.viewmodel.BrowserSortField
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserState
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserViewModel
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SyncGlobalState
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SyncSnapshotUi
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SyncViewModel
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SetupPhase
import io.github.yuanbaobaoo.petallink.ui.viewmodel.TransferTaskUi
import io.github.yuanbaobaoo.petallink.ui.viewmodel.UpdaterPhase
import io.github.yuanbaobaoo.petallink.auth.UserInfo
import io.github.yuanbaobaoo.petallink.ui.viewmodel.TransferViewModel
import io.github.yuanbaobaoo.petallink.update.UpdateManifest
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

enum class AppPage { LOGIN, FILES, SETTINGS, LOGS }

/**
 * 桌面 UI 全量状态。
 *
 * sync 为完整同步快照（对标原 Vue sync store 全部字段）；
 * setupPhase 由 mountConfigured 派生，驱动 SyncSetupBanner 三态。
 */
data class DesktopUiState(
    val initialized: Boolean = false,
    val page: AppPage = AppPage.LOGIN,
    val loggedIn: Boolean = false,
    val loggingIn: Boolean = false,
    val secretConfigured: Boolean = true,
    val netState: NetState = NetState.OFFLINE,
    val syncStatus: String = "初始化中",
    val sync: SyncSnapshotUi = SyncSnapshotUi(),
    val setupPhase: SetupPhase = SetupPhase.LOADING,
    val config: UserConfig = UserConfig(),
    val transfers: List<TransferTaskUi> = emptyList(),
    val browser: FileBrowserState = FileBrowserState(),
    val thumbnails: Map<String, ByteArray> = emptyMap(),
    val fileStatuses: Map<String, String> = emptyMap(),
    val quotaText: String? = null,
    val logs: List<LogRecordDisplay> = emptyList(),
    val userInfo: UserInfo? = null,
    val userName: String? = null,
    val appVersion: String = "",
    val availableUpdate: UpdateManifest? = null,
    val updateStatus: String = "",
    val updatePhase: UpdaterPhase = UpdaterPhase.IDLE,
    val updateDownloadProgress: Float = 0f,
    val updateReadyToQuit: Boolean = false,
    val launchAtLogin: Boolean = false,
    val errorMessage: String? = null,
    val sidebarRefresh: Int = 0,
)

/** 桌面 UI 的真实状态适配层；所有 IO 都通过 Composition Root 中的服务执行。 */
class DesktopAppViewModel(
    private val scope: CoroutineScope,
    private val commands: CommandService,
    private val netGuard: NetGuard,
) {
    private val mutableState = MutableStateFlow(DesktopUiState())
    private val browser = FileBrowserViewModel()
    private val syncViewModel = SyncViewModel()
    private val transferViewModel = TransferViewModel()
    private val transferRequest = AtomicInteger()
    private val thumbnailRequests = mutableSetOf<String>()
    private val updateCheckMutex = Mutex()
    private val automaticUpdateChecksStarted = AtomicBoolean(false)
    private var lastAutomaticUpdateCheckMs: Long? = null
    private var lastNetState: NetState? = null
    val state: StateFlow<DesktopUiState> = mutableState.asStateFlow()

    init {
        scope.launch {
            browser.state.collect { value -> mutableState.value = mutableState.value.copy(browser = value) }
        }
        scope.launch {
            transferViewModel.tasks.collect { tasks ->
                mutableState.value = mutableState.value.copy(transfers = tasks)
            }
        }
        scope.launch {
            syncViewModel.state.collect { value ->
                mutableState.value = mutableState.value.copy(syncStatus = when (value) {
                    SyncGlobalState.IDLE -> "空闲"
                    SyncGlobalState.INDEXING -> "正在索引"
                    SyncGlobalState.SYNCING -> "正在同步"
                    SyncGlobalState.PAUSED -> "离线暂停"
                    SyncGlobalState.ERROR -> "同步异常"
                })
            }
        }
        scope.launch {
            syncViewModel.snapshot.collect { snap ->
                mutableState.value = mutableState.value.copy(
                    sync = snap,
                    sidebarRefresh = syncViewModel.sidebarRefresh.value,
                )
            }
        }
        scope.launch {
            var lastContentRevision = 0L
            commands.syncStates.collect { snapshot ->
                // 透出完整快照（counts + runtime + failedItems + syncPhase），不再压缩成字符串。
                val ui = SyncSnapshotUi(
                    revision = snapshot.revision,
                    global = when (snapshot.global) {
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.IDLE -> SyncGlobalState.IDLE
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.INDEXING -> SyncGlobalState.INDEXING
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.SYNCING -> SyncGlobalState.SYNCING
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.PAUSED -> SyncGlobalState.PAUSED
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.ERROR -> SyncGlobalState.ERROR
                    },
                    total = snapshot.counts.total,
                    completed = snapshot.counts.completed,
                    uploading = snapshot.counts.uploading,
                    downloading = snapshot.counts.downloading,
                    waitingNetwork = snapshot.counts.waitingNetwork,
                    failed = snapshot.counts.failed,
                    transferFailed = snapshot.counts.transferFailed,
                    conflict = snapshot.counts.conflict,
                    editing = snapshot.runtime.editing,
                    isRunning = snapshot.runtime.isRunning,
                    isIndexing = snapshot.runtime.isIndexing,
                    indexingScannedFolders = snapshot.runtime.indexingScannedFolders.toLong(),
                    indexingDiscoveredItems = snapshot.runtime.indexingDiscoveredItems.toLong(),
                    syncPhase = snapshot.runtime.syncPhase,
                    lastSyncTime = snapshot.runtime.lastSyncTime,
                    contentChanged = snapshot.runtime.contentChanged,
                    failedItems = snapshot.failedItems.map {
                        io.github.yuanbaobaoo.petallink.ui.viewmodel.FailedItemUi(it.relativePath, it.errorMessage)
                    },
                )
                syncViewModel.applySnapshot(ui)
                if (snapshot.runtime.contentChanged && snapshot.revision > lastContentRevision && mutableState.value.loggedIn) {
                    lastContentRevision = snapshot.revision
                    refreshInternal(triggerSync = false)
                }
            }
        }
        commands.folderSyncProgress?.let { progress ->
            scope.launch {
                progress.collect { value ->
                    if (value != null) {
                        mutableState.value = mutableState.value.copy(
                            syncStatus = if (value.done < value.total) {
                                "目录同步 ${value.done}/${value.total}"
                            } else "目录同步完成 ${value.done}/${value.total}",
                        )
                    }
                }
            }
        }
        commands.uploadFailures?.let { failures ->
            scope.launch {
                failures.collect { failure ->
                    mutableState.value = mutableState.value.copy(
                        errorMessage = "${failure.relativePath}：${failure.message}",
                    )
                }
            }
        }
        scope.launch {
            netGuard.state.collect { net ->
                if (lastNetState == NetState.OFFLINE && net == NetState.ONLINE) {
                    commands.syncNetworkRecovered()
                }
                lastNetState = net
                syncViewModel.updateNetState(net)
                mutableState.value = mutableState.value.copy(netState = net)
            }
        }
        scope.launch { restore() }
        scope.launch {
            delay(STARTUP_UPDATE_DELAY_MS)
            checkForUpdateInternal(automatic = true, minimumIntervalMs = 0L)
            automaticUpdateChecksStarted.set(true)
            while (isActive) {
                delay(PERIODIC_UPDATE_INTERVAL_MS)
                checkForUpdateInternal(automatic = true, minimumIntervalMs = PERIODIC_UPDATE_INTERVAL_MS)
            }
        }
    }

    private suspend fun restore() {
        val config = when (val result = commands.configLoad()) {
            is AppResult.Ok -> result.value
            is AppResult.Err -> {
                mutableState.value = mutableState.value.copy(
                    initialized = true,
                    syncStatus = "配置加载失败",
                    errorMessage = result.error.message,
                )
                return
            }
        }
        val authState = (commands.authRestore() as? AppResult.Ok)?.value
        val loggedIn = authState?.loggedIn == true
        mutableState.value = mutableState.value.copy(
            initialized = true,
            loggedIn = loggedIn,
            page = if (loggedIn) AppPage.FILES else AppPage.LOGIN,
            syncStatus = if (loggedIn) "空闲" else "等待登录",
            config = config,
            setupPhase = deriveSetupPhase(config.mountConfigured),
            secretConfigured = authState?.secretConfigured ?: commands.authCheckSecret(),
            launchAtLogin = commands.platformLaunchAtLoginIsEnabled(),
            appVersion = commands.platformAppGetVersion(),
            errorMessage = null,
        )
        if (loggedIn) refresh()
        if (loggedIn) loadUserInfo()
    }

    fun login() {
        if (!mutableState.value.secretConfigured) {
            mutableState.value = mutableState.value.copy(errorMessage = "缺少华为 OAuth client secret")
            return
        }
        if (mutableState.value.loggingIn) return
        mutableState.value = mutableState.value.copy(loggingIn = true, errorMessage = null)
        scope.launch {
            when (val result = commands.authLogin(mutableState.value.config.oauthCallbackPort)) {
                is AppResult.Ok -> {
                    val resetConfig = (commands.configLoad() as? AppResult.Ok)?.value ?: UserConfig()
                    mutableState.value = mutableState.value.copy(
                        loggedIn = true,
                        loggingIn = false,
                        page = if (resetConfig.mountConfigured) AppPage.FILES else AppPage.SETTINGS,
                        syncStatus = if (resetConfig.mountConfigured) "空闲" else "请选择同步目录",
                        config = resetConfig,
                        setupPhase = deriveSetupPhase(resetConfig.mountConfigured),
                    )
                    loadUserInfo()
                    if (resetConfig.mountConfigured) refresh()
                }
                is AppResult.Err -> mutableState.value = mutableState.value.copy(
                    loggingIn = false,
                    errorMessage = result.error.message,
                )
            }
        }
    }

    fun refresh() = refreshInternal(triggerSync = true)

    private fun refreshInternal(triggerSync: Boolean) {
        if (!mutableState.value.loggedIn) return
        mutableState.value = mutableState.value.copy(syncStatus = "刷新中", errorMessage = null)
        val requestId = browser.beginLoad()
        val folderId = browser.state.value.folderId
        val transferRequestId = transferRequest.incrementAndGet()
        scope.launch {
            if (triggerSync && mutableState.value.config.mountConfigured) commands.syncManualRefresh()
            val fileResult = commands.driveList(folderId, null, 100)
            val transferResult = commands.transferListAll()
            (fileResult as? AppResult.Ok)?.value?.let {
                browser.applyPage(requestId, folderId, it.files, it.nextCursor, append = false)
                val ids = it.files.mapNotNull(DriveFile::id)
                val statuses = (commands.syncBatchFileStatus(ids) as? AppResult.Ok)?.value.orEmpty()
                mutableState.value = mutableState.value.copy(fileStatuses = statuses)
            }
            val transferTasks = (transferResult as? AppResult.Ok)?.value
            transferTasks?.let { tasks ->
                transferViewModel.loadAll(tasks.map { task ->
                    TransferTaskUi(
                        id = task.id ?: return@map null,
                        fileName = task.name,
                        state = task.state,
                        stateRevision = task.stateRevision,
                        bytesTotal = task.bytesTotal,
                        bytesDone = task.bytesDone,
                        direction = when (task.direction) {
                            TransferDirection.UPLOAD -> "upload"
                            TransferDirection.DOWNLOAD, TransferDirection.DOWNLOAD_UPDATE -> "download"
                            TransferDirection.DELETE -> "delete"
                        },
                        errorMessage = task.errorMessage,
                    )
                }.filterNotNull(), transferRequestId)
            }
            (commands.syncSnapshot() as? AppResult.Ok)?.value?.let { snapshot ->
                syncViewModel.applyState(
                    when (snapshot.global) {
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.IDLE -> SyncGlobalState.IDLE
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.INDEXING -> SyncGlobalState.INDEXING
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.SYNCING -> SyncGlobalState.SYNCING
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.PAUSED -> SyncGlobalState.PAUSED
                        io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus.ERROR -> SyncGlobalState.ERROR
                    },
                    snapshot.revision,
                )
            }
            val error = (fileResult as? AppResult.Err)?.error?.message
                ?: (transferResult as? AppResult.Err)?.error?.message
            mutableState.value = mutableState.value.copy(
                syncStatus = if (error == null) "已刷新" else "刷新失败",
                errorMessage = error,
            )
        }
    }

    fun loadMore() {
        val current = browser.state.value
        val cursor = current.nextCursor ?: return
        val requestId = browser.beginLoad()
        scope.launch {
            when (val result = commands.driveList(current.folderId, cursor, 100)) {
                is AppResult.Ok -> browser.applyPage(
                    requestId, current.folderId, result.value.files, result.value.nextCursor, append = true,
                )
                is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
            }
        }
    }

    fun enterFolder(folder: DriveFile) {
        browser.enter(folder)
        refresh()
    }

    fun navigateTo(breadcrumb: BrowserBreadcrumb) {
        browser.navigateTo(breadcrumb)
        refresh()
    }

    fun search(query: String) {
        browser.search(query)
        if (query.isBlank()) return refresh()
        val request = browser.beginLoad()
        val folder = browser.state.value.folderId
        scope.launch {
            when (val result = commands.driveSearch(query, folder, 100)) {
                is AppResult.Ok -> browser.applyPage(request, folder, result.value.files, result.value.nextCursor, false)
                is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
            }
        }
    }

    fun sort(field: BrowserSortField) = browser.sort(field)

    fun loadThumbnail(file: DriveFile) {
        val id = file.id ?: return
        if (file.thumbnailLink.isNullOrBlank() || id in mutableState.value.thumbnails || !thumbnailRequests.add(id)) return
        scope.launch {
            when (val result = commands.driveGetThumbnail(id)) {
                is AppResult.Ok -> mutableState.value = mutableState.value.copy(
                    thumbnails = mutableState.value.thumbnails + (id to result.value),
                )
                is AppResult.Err -> Unit
            }
            thumbnailRequests.remove(id)
        }
    }

    fun openItem(file: DriveFile) {
        if (file.isFolder()) return enterFolder(file)
        val id = file.id ?: return
        val destination = localPathFor(file)
        scope.launch {
            when (val result = commands.syncDownloadOnDemand(id, destination.toString())) {
                is AppResult.Ok -> mutableState.value = mutableState.value.copy(syncStatus = "已下载 ${displayName(file)}")
                is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
            }
            refresh()
        }
    }

    fun deleteItems(files: List<DriveFile>) = scope.launch {
        val errors = files.mapNotNull { file ->
            val id = file.id ?: return@mapNotNull "${displayName(file)} 缺少 id"
            (commands.driveDeleteFile(id, displayName(file)) as? AppResult.Err)?.error?.message
        }
        mutableState.value = mutableState.value.copy(errorMessage = errors.firstOrNull())
        refresh()
    }

    fun createFolder(name: String) = scope.launch {
        when (val result = commands.driveCreateFolder(name.trim(), browser.state.value.folderId)) {
            is AppResult.Ok -> refresh()
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun renameItem(file: DriveFile, newName: String) = scope.launch {
        val id = file.id ?: return@launch
        when (val result = commands.driveRenameFile(id, newName.trim())) {
            is AppResult.Ok -> refresh()
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun moveItem(file: DriveFile, newParentId: String) = scope.launch {
        val id = file.id ?: return@launch
        when (val result = commands.driveMoveFile(id, newParentId.trim())) {
            is AppResult.Ok -> refresh()
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun syncFolder(file: DriveFile) = scope.launch {
        val id = file.id ?: return@launch
        when (val result = commands.syncFolderRecursive(id, relativePathFor(file))) {
            is AppResult.Ok -> mutableState.value = mutableState.value.copy(syncStatus = "已检查 ${result.value} 项")
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
        refresh()
    }

    fun chooseAndUpload() {
        val dialog = java.awt.FileDialog(null as java.awt.Frame?, "选择上传文件", java.awt.FileDialog.LOAD)
        dialog.isMultipleMode = false
        dialog.isVisible = true
        val file = dialog.files.firstOrNull()?.toPath() ?: return
        scope.launch {
            when (val result = commands.driveUploadFile(file.toString(), browser.state.value.folderId)) {
                is AppResult.Ok -> refresh()
                is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
            }
        }
    }

    fun openMountInFinder() = scope.launch {
        val dir = mutableState.value.config.mountDir
        if (dir.isBlank()) mutableState.value = mutableState.value.copy(errorMessage = "尚未配置同步目录")
        else commands.platformOpenInFinder(dir)
    }

    fun loadQuota() = scope.launch {
        when (val result = commands.driveGetAbout()) {
            is AppResult.Ok -> mutableState.value = mutableState.value.copy(
                quotaText = "${formatBytes(result.value.usedBytes())} / ${formatBytes(result.value.totalBytes())}",
            )
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun canFreeUp(file: DriveFile, onResult: (Boolean) -> Unit) = scope.launch {
        val id = file.id
        if (id == null || file.isFolder()) {
            onResult(file.isFolder() && mutableState.value.config.mountConfigured)
            return@launch
        }
        val result = commands.syncCheckSafeFreeUp(relativePathFor(file), id)
        onResult((result as? AppResult.Ok)?.value == "safe")
    }

    fun freeUpItems(files: List<DriveFile>) = scope.launch {
        val items = files.flatMap { file ->
            if (file.isFolder()) {
                (commands.syncListFreeableInFolder(relativePathFor(file)) as? AppResult.Ok)?.value.orEmpty()
            } else {
                val id = file.id ?: return@flatMap emptyList()
                listOf(io.github.yuanbaobaoo.petallink.commands.FreeableItem(
                    id, relativePathFor(file), localPathFor(file).toString(), displayName(file), file.sizeBytes,
                ))
            }
        }.distinctBy { it.fileId }
        when (val result = commands.syncFreeUpBatch(items)) {
            is AppResult.Ok -> mutableState.value = mutableState.value.copy(
                syncStatus = "已释放 ${result.value.freedCount} 项（${formatBytes(result.value.freedBytes)}）",
                errorMessage = result.value.errors.firstOrNull(),
            )
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
        refresh()
    }

    fun openLogs() {
        mutableState.value = mutableState.value.copy(page = AppPage.LOGS)
        reloadLogs()
    }

    fun reloadLogs() {
        val records = (commands.platformLogsList() as? AppResult.Ok)?.value.orEmpty().map {
            LogRecordDisplay(it.timestampMs, it.level, it.target, it.message)
        }.asReversed()
        mutableState.value = mutableState.value.copy(logs = records)
    }

    fun clearLogs() {
        commands.platformLogsClear()
        reloadLogs()
    }

    fun retryTransfer(taskId: Long) = scope.launch {
        when (val result = commands.transferRetry(taskId)) {
            is AppResult.Ok -> refresh()
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun clearFinishedTransfers() = scope.launch {
        commands.transferClearFinished()
        refresh()
    }

    /** 仅清除已完成（Completed）任务（对标原 Vue transfer_clear_completed）。 */
    fun clearCompletedTransfers() = scope.launch {
        commands.transferClearCompleted()
        refresh()
    }

    /** 仅清除失败历史（Failed）任务（对标原 Vue transfer_clear_failed）。 */
    fun clearFailedTransfers() = scope.launch {
        commands.transferClearFailed()
        refresh()
    }

    fun exportLogs() {
        val target = Path.of(System.getProperty("user.home"), "Desktop", "PetalLink-logs.txt")
        when (val result = commands.platformLogsExport(target.toString())) {
            is AppResult.Ok -> mutableState.value = mutableState.value.copy(syncStatus = "日志已导出到 $target")
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun exportConfig() {
        val target = Path.of(System.getProperty("user.home"), "Desktop", "PetalLink-config.json")
        when (val result = commands.configExportJson()) {
            is AppResult.Ok -> runCatching { java.nio.file.Files.writeString(target, result.value) }
                .onSuccess { mutableState.value = mutableState.value.copy(syncStatus = "配置已导出到 $target") }
                .onFailure { mutableState.value = mutableState.value.copy(errorMessage = it.message) }
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun importConfig() {
        val dialog = java.awt.FileDialog(null as java.awt.Frame?, "导入配置", java.awt.FileDialog.LOAD)
        dialog.isVisible = true
        val file = dialog.files.firstOrNull()?.toPath() ?: return
        val parsed = runCatching { java.nio.file.Files.readString(file) }.getOrElse {
            mutableState.value = mutableState.value.copy(errorMessage = it.message)
            return
        }
        when (val result = commands.configImportJson(parsed)) {
            is AppResult.Ok -> {
                mutableState.value = mutableState.value.copy(config = result.value, syncStatus = "配置已导入")
            }
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun clearApplicationCache() = scope.launch {
        when (val result = commands.platformClearCache()) {
            is AppResult.Ok -> mutableState.value = DesktopUiState(initialized = true, syncStatus = "缓存已清理，请重新登录")
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    fun checkForUpdate() = scope.launch { checkForUpdateInternal(automatic = false, minimumIntervalMs = 0L) }

    /** 窗口获焦触发静默检查；首次启动检查完成前忽略，之后按 10 分钟节流。 */
    fun onWindowFocused() {
        if (!automaticUpdateChecksStarted.get()) return
        scope.launch { checkForUpdateInternal(automatic = true, minimumIntervalMs = FOCUS_UPDATE_INTERVAL_MS) }
    }

    private suspend fun checkForUpdateInternal(automatic: Boolean, minimumIntervalMs: Long) {
        updateCheckMutex.withLock {
            val now = System.currentTimeMillis()
            val last = lastAutomaticUpdateCheckMs
            if (automatic && last != null && now - last < minimumIntervalMs) return@withLock
            if (automatic) lastAutomaticUpdateCheckMs = now
            if (!automatic) {
                mutableState.value = mutableState.value.copy(
                    updateStatus = "正在检查更新…",
                    updatePhase = UpdaterPhase.CHECKING,
                    errorMessage = null,
                )
            }
            when (val result = commands.updaterCheck()) {
                is AppResult.Ok -> mutableState.value = mutableState.value.copy(
                    availableUpdate = result.value,
                    updateStatus = result.value?.let { "发现新版本 ${it.version}" }
                        ?: if (automatic) "" else "已是最新版本",
                    updatePhase = if (result.value != null) UpdaterPhase.AVAILABLE else UpdaterPhase.IDLE,
                )
                is AppResult.Err -> if (!automatic) {
                    mutableState.value = mutableState.value.copy(
                        updateStatus = "检查更新失败",
                        updatePhase = UpdaterPhase.FAILED,
                        errorMessage = result.error.message,
                    )
                }
            }
        }
    }

    fun installUpdate() {
        val manifest = mutableState.value.availableUpdate ?: return
        scope.launch {
            mutableState.value = mutableState.value.copy(
                updateStatus = "等待传输并下载更新…",
                updatePhase = UpdaterPhase.DOWNLOADING,
                updateDownloadProgress = 0f,
                errorMessage = null,
            )
            when (val result = commands.updaterDownloadAndInstall(manifest)) {
                is AppResult.Ok -> mutableState.value = mutableState.value.copy(
                    updateStatus = "更新已校验，正在重启安装",
                    updatePhase = UpdaterPhase.READY,
                    updateReadyToQuit = result.value,
                )
                is AppResult.Err -> mutableState.value = mutableState.value.copy(
                    updateStatus = "更新失败",
                    updatePhase = UpdaterPhase.FAILED,
                    errorMessage = result.error.message,
                )
            }
        }
    }

    /** 关闭更新弹窗（回到 IDLE，但保留 availableUpdate 以便侧边栏提示）。 */
    fun dismissUpdateDialog() {
        mutableState.value = mutableState.value.copy(updatePhase = UpdaterPhase.IDLE)
    }

    fun logout() = scope.launch {
        when (val result = commands.authLogout()) {
            is AppResult.Ok -> mutableState.value = DesktopUiState(initialized = true, syncStatus = "等待登录")
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    private fun loadUserInfo() = scope.launch {
        val user = (commands.authGetUserInfo() as? AppResult.Ok)?.value
        mutableState.value = mutableState.value.copy(
            userInfo = user,
            userName = user?.displayName ?: user?.nickname,
        )
    }

    /**
     * 派生同步目录配置阶段（对标原 Vue sync.setupPhase）。
     *
     * mountConfigured=false → NEEDS_SETUP；=true 但从未成功同步 → NEEDS_FIRST_SYNC；否则 ACTIVE。
     */
    private fun deriveSetupPhase(mountConfigured: Boolean): SetupPhase {
        if (!mountConfigured) return SetupPhase.NEEDS_SETUP
        val snap = mutableState.value.sync
        // 已有过同步内容（基线非空）或上次同步时间存在 → ACTIVE，否则等待首次同步
        return if (snap.total > 0 || snap.lastSyncTime != null) SetupPhase.ACTIVE else SetupPhase.NEEDS_FIRST_SYNC
    }

    private fun relativePathFor(file: DriveFile): String =
        (browser.state.value.breadcrumbs.drop(1).map { it.name } + displayName(file)).joinToString("/")

    private fun localPathFor(file: DriveFile): Path {
        val root = io.github.yuanbaobaoo.petallink.config.JvmMountPaths.resolve(mutableState.value.config.mountDir)
        return root.resolve(relativePathFor(file)).normalize().also {
            require(it.startsWith(root)) { "文件路径越界" }
        }
    }

    private fun displayName(file: DriveFile): String = file.name ?: file.fileName ?: "未命名"

    fun cancelLogin() {
        if (!mutableState.value.loggingIn) return
        scope.launch {
            commands.authCancelLogin()
            mutableState.value = mutableState.value.copy(loggingIn = false, errorMessage = null)
        }
    }

    fun setLaunchAtLogin(enabled: Boolean): Boolean {
        val changed = commands.platformLaunchAtLoginSetEnabled(enabled)
        if (changed) mutableState.value = mutableState.value.copy(launchAtLogin = enabled)
        return changed
    }

    fun openSettings() {
        mutableState.value = mutableState.value.copy(page = AppPage.SETTINGS)
    }

    fun openFiles() {
        mutableState.value = mutableState.value.copy(page = AppPage.FILES)
    }

    fun saveConfig(config: UserConfig): List<String> {
        val errors = ConfigValidator.validate(config)
        if (errors.isNotEmpty()) return errors
        return when (val result = commands.configSave(config)) {
            is AppResult.Ok -> {
                mutableState.value = mutableState.value.copy(config = config, page = AppPage.FILES)
                emptyList()
            }
            is AppResult.Err -> listOf(result.error.message ?: "配置保存失败")
        }
    }

    private fun formatBytes(bytes: Long): String = when {
        bytes >= 1_073_741_824 -> "%.1f GB".format(bytes / 1_073_741_824.0)
        bytes >= 1_048_576 -> "%.1f MB".format(bytes / 1_048_576.0)
        bytes >= 1024 -> "%.1f KB".format(bytes / 1024.0)
        else -> "$bytes B"
    }

    companion object {
        const val STARTUP_UPDATE_DELAY_MS = 3_000L
        const val PERIODIC_UPDATE_INTERVAL_MS = 60L * 60 * 1_000
        const val FOCUS_UPDATE_INTERVAL_MS = 10L * 60 * 1_000
    }
}

/** 应用级 Composition Root，负责长生命周期对象的创建与逆序关闭。 */
class ApplicationRoot(val paths: AppPaths = AppPaths.fromEnvironment()) : AutoCloseable {
    private val closed = AtomicBoolean(false)
    private val logger = Logger()
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val commands = CommandService.create(paths)
    val netGuard = NetGuard(scope)
    val viewModel = DesktopAppViewModel(scope, commands, netGuard)

    init {
        logger.info("app.root") { "应用服务已装配，dataDir=${paths.dataDir}" }
        netGuard.startProbe()
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        netGuard.stopProbe()
        scope.cancel("application closing")
        commands.close()
        logger.info("app.root") { "应用服务已关闭" }
    }
}
