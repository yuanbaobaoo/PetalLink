package io.github.yuanbaobaoo.petallink.app

import io.github.yuanbaobaoo.petallink.commands.AppResult
import io.github.yuanbaobaoo.petallink.commands.CommandService
import io.github.yuanbaobaoo.petallink.config.ConfigValidator
import io.github.yuanbaobaoo.petallink.config.SortField
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.core.logging.Logger
import io.github.yuanbaobaoo.petallink.core.net_guard.NetGuard
import io.github.yuanbaobaoo.petallink.core.net_guard.NetState
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.drive.displayName
import io.github.yuanbaobaoo.petallink.drive.totalBytes
import io.github.yuanbaobaoo.petallink.drive.usedBytes
import io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus
import io.github.yuanbaobaoo.petallink.sync.isFolder
import io.github.yuanbaobaoo.petallink.ui.pages.main.LogRecordDisplay
import io.github.yuanbaobaoo.petallink.ui.viewmodel.BrowserBreadcrumb
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserState
import io.github.yuanbaobaoo.petallink.ui.viewmodel.FileBrowserViewModel
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

/**
 * 应用主页面路由：登录、文件、设置、日志。
 */
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

/**
 * 桌面 UI 的真实状态适配层；所有 IO 都通过 Composition Root 中的服务执行。
 */
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
                    SyncGlobalStatus.IDLE -> "空闲"
                    SyncGlobalStatus.INDEXING -> "正在索引"
                    SyncGlobalStatus.SYNCING -> "正在同步"
                    SyncGlobalStatus.PAUSED -> "离线暂停"
                    SyncGlobalStatus.ERROR -> "同步异常"
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
                    global = snapshot.global,
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
                    failedItems = snapshot.failedItems,
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
            commands.transferUpdates.collect { reloadTransfers() }
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

    /**
     * 恢复应用启动状态：加载配置、尝试恢复登录态，并按需触发刷新与用户信息加载。
     */
    private suspend fun restore() {
        val authState = when (val result = commands.authRestore()) {
            is AppResult.Ok -> result.value
            is AppResult.Err -> {
                mutableState.value = mutableState.value.copy(
                    initialized = true,
                    syncStatus = "登录状态恢复失败",
                    errorMessage = result.error.message,
                )
                return
            }
        }
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
        val loggedIn = authState.loggedIn
        mutableState.value = mutableState.value.copy(
            initialized = true,
            loggedIn = loggedIn,
            page = if (loggedIn) AppPage.FILES else AppPage.LOGIN,
            syncStatus = if (loggedIn) "空闲" else "等待登录",
            config = config,
            setupPhase = deriveSetupPhase(config.mountConfigured),
            secretConfigured = authState.secretConfigured,
            launchAtLogin = commands.platformLaunchAtLoginIsEnabled(),
            appVersion = commands.platformAppGetVersion(),
            errorMessage = null,
        )
        if (loggedIn) refresh()
        if (loggedIn) loadUserInfo()
    }

    /**
     * 触发用户登录：校验 secret 配置后调用 OAuth 登录，成功后重载配置并进入文件页或设置页。
     */
    fun login() {
        if (!mutableState.value.secretConfigured) {
            mutableState.value = mutableState.value.copy(errorMessage = "缺少华为 OAuth client_id 或 client_secret")
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

    /**
     * 刷新当前文件列表，并触发一次手动同步。
     */
    fun refresh() = refreshInternal(triggerSync = true)

    /**
     * 刷新内部实现：重载当前目录文件、传输任务、文件状态与同步快照，并更新 UI 状态。
     */
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
                    snapshot.global,
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

    /**
     * 从持久任务队列重载传输视图；请求序号保证较旧查询不会覆盖较新结果。
     */
    private suspend fun reloadTransfers() {
        val requestId = transferRequest.incrementAndGet()
        val tasks = (commands.transferListAll() as? AppResult.Ok)?.value ?: return
        transferViewModel.loadAll(tasks.mapNotNull { task ->
            TransferTaskUi(
                id = task.id ?: return@mapNotNull null,
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
        }, requestId)
    }

    /**
     * 加载更多文件：基于分页游标追加下一页结果。
     */
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

    /**
     * 进入指定文件夹并刷新其内容。
     */
    fun enterFolder(folder: DriveFile) {
        browser.enter(folder)
        refresh()
    }

    /**
     * 根据面包屑导航到指定层级并刷新。
     */
    fun navigateTo(breadcrumb: BrowserBreadcrumb) {
        browser.navigateTo(breadcrumb)
        refresh()
    }

    /**
     * 按关键字搜索当前目录文件；查询为空时回到普通刷新。
     */
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

    /**
     * 切换文件列表排序字段。
     */
    fun sort(field: SortField) = browser.sort(field)

    /**
     * 异步加载文件缩略图，已加载或请求中的不重复拉取。
     */
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

    /**
     * 打开文件项：文件夹则进入；仅在已配置同步目录时，文件才按需下载到本地后刷新。
     */
    fun openItem(file: DriveFile) {
        if (file.isFolder()) return enterFolder(file)
        if (!mutableState.value.config.mountConfigured) return
        val id = file.id ?: return
        val destination = localPathFor(file)
        scope.launch {
            when (val result = commands.syncDownloadOnDemand(id, destination.toString())) {
                is AppResult.Ok -> mutableState.value = mutableState.value.copy(syncStatus = "已下载 ${file.displayName()}")
                is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
            }
            refresh()
        }
    }

    /**
     * 批量删除指定文件项，首个错误回写到 UI，完成后刷新。
     */
    fun deleteItems(files: List<DriveFile>) = scope.launch {
        val errors = files.mapNotNull { file ->
            val id = file.id ?: return@mapNotNull "${file.displayName()} 缺少 id"
            (commands.driveDeleteFile(id, file.displayName()) as? AppResult.Err)?.error?.message
        }
        mutableState.value = mutableState.value.copy(errorMessage = errors.firstOrNull())
        refresh()
    }

    /**
     * 在当前目录下创建新文件夹。
     */
    fun createFolder(name: String) = scope.launch {
        when (val result = commands.driveCreateFolder(name.trim(), browser.state.value.folderId)) {
            is AppResult.Ok -> refresh()
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 重命名指定文件项。
     */
    fun renameItem(file: DriveFile, newName: String) = scope.launch {
        val id = file.id ?: return@launch
        when (val result = commands.driveRenameFile(id, newName.trim())) {
            is AppResult.Ok -> refresh()
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 将指定文件项移动到新的父目录。
     */
    fun moveItem(file: DriveFile, newParentId: String) = scope.launch {
        val id = file.id ?: return@launch
        when (val result = commands.driveMoveFile(id, newParentId.trim())) {
            is AppResult.Ok -> refresh()
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 递归检查并同步指定文件夹下的内容。
     */
    fun syncFolder(file: DriveFile) = scope.launch {
        val id = file.id ?: return@launch
        when (val result = commands.syncFolderRecursive(id, relativePathFor(file))) {
            is AppResult.Ok -> mutableState.value = mutableState.value.copy(syncStatus = "已检查 ${result.value} 项")
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
        refresh()
    }

    /**
     * 弹出文件选择对话框，将所选本地文件上传到当前目录。
     */
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

    /**
     * 使用 macOS 原生目录选择器获取同步根目录，并把结果交给调用方决定何时保存配置。
     */
    fun chooseMountDirectory(onSelected: (String) -> Unit) {
        val property = "apple.awt.fileDialogForDirectories"
        val previous = System.getProperty(property)
        try {
            System.setProperty(property, "true")
            val dialog = java.awt.FileDialog(null as java.awt.Frame?, "选择同步目录", java.awt.FileDialog.LOAD)
            dialog.isMultipleMode = false
            dialog.isVisible = true
            dialog.files.firstOrNull()?.toPath()?.toAbsolutePath()?.normalize()?.toString()?.let(onSelected)
        } finally {
            if (previous == null) System.clearProperty(property) else System.setProperty(property, previous)
        }
    }

    /**
     * 在系统资源管理器中打开同步挂载目录。
     */
    fun openMountInFinder() = scope.launch {
        val dir = mutableState.value.config.mountDir
        if (dir.isBlank()) mutableState.value = mutableState.value.copy(errorMessage = "尚未配置同步目录")
        else commands.platformOpenInFinder(dir)
    }

    /**
     * 查询云盘配额并更新为"已用 / 总量"文本。
     */
    fun loadQuota() = scope.launch {
        when (val result = commands.driveGetAbout()) {
            is AppResult.Ok -> mutableState.value = mutableState.value.copy(
                quotaText = "${formatBytes(result.value.usedBytes())} / ${formatBytes(result.value.totalBytes())}",
            )
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 检查指定文件是否可安全释放本地空间，结果通过回调返回。
     */
    fun canFreeUp(file: DriveFile, onResult: (Boolean) -> Unit) = scope.launch {
        val id = file.id
        if (id == null || file.isFolder()) {
            onResult(file.isFolder() && mutableState.value.config.mountConfigured)
            return@launch
        }
        val result = commands.syncCheckSafeFreeUp(relativePathFor(file), id)
        onResult((result as? AppResult.Ok)?.value == "safe")
    }

    /**
     * 展开所选文件和目录，生成去重后的释放空间预览项。
     */
    fun previewFreeUpItems(
        files: List<DriveFile>,
        onResult: (List<io.github.yuanbaobaoo.petallink.commands.FreeableItem>) -> Unit,
    ) = scope.launch {
        val items = files.flatMap { file ->
            if (file.isFolder()) {
                (commands.syncListFreeableInFolder(relativePathFor(file)) as? AppResult.Ok)?.value.orEmpty()
            } else {
                val id = file.id ?: return@flatMap emptyList()
                listOf(io.github.yuanbaobaoo.petallink.commands.FreeableItem(
                    id, relativePathFor(file), localPathFor(file).toString(), file.displayName(), file.sizeBytes,
                ))
            }
        }.distinctBy { it.fileId }
        onResult(items)
    }

    /**
     * 执行用户已确认的释放空间预览项，完成后刷新同步状态。
     */
    fun freeUpItems(items: List<io.github.yuanbaobaoo.petallink.commands.FreeableItem>) = scope.launch {
        when (val result = commands.syncFreeUpBatch(items)) {
            is AppResult.Ok -> mutableState.value = mutableState.value.copy(
                syncStatus = "已释放 ${result.value.freedCount} 项（${formatBytes(result.value.freedBytes)}）",
                errorMessage = result.value.errors.firstOrNull(),
            )
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
        refresh()
    }

    /**
     * 切换到日志页并重新加载日志列表。
     */
    fun openLogs() {
        mutableState.value = mutableState.value.copy(page = AppPage.LOGS)
        reloadLogs()
    }

    /**
     * 重新加载日志记录并按倒序展示。
     */
    fun reloadLogs() {
        val records = (commands.platformLogsList() as? AppResult.Ok)?.value.orEmpty().map {
            LogRecordDisplay(it.timestampMs, it.level, it.target, it.message)
        }.asReversed()
        mutableState.value = mutableState.value.copy(logs = records)
    }

    /**
     * 清空全部日志并重新加载（此时为空）。
     */
    fun clearLogs() {
        commands.platformLogsClear()
        reloadLogs()
    }

    /**
     * 重试指定的传输任务，完成后刷新。
     */
    fun retryTransfer(taskId: Long) = scope.launch {
        when (val result = commands.transferRetry(taskId)) {
            is AppResult.Ok -> refresh()
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 清除所有已完成的传输任务记录。
     */
    fun clearFinishedTransfers() = scope.launch {
        commands.transferClearFinished()
        refresh()
    }

    /**
     * 仅清除已完成（Completed）任务（对标原 Vue transfer_clear_completed）。
     */
    fun clearCompletedTransfers() = scope.launch {
        commands.transferClearCompleted()
        refresh()
    }

    /**
     * 仅清除失败历史（Failed）任务（对标原 Vue transfer_clear_failed）。
     */
    fun clearFailedTransfers() = scope.launch {
        commands.transferClearFailed()
        refresh()
    }

    /**
     * 将日志导出到桌面 PetalLink-logs.txt 文件。
     */
    fun exportLogs() {
        val target = Path.of(System.getProperty("user.home"), "Desktop", "PetalLink-logs.txt")
        when (val result = commands.platformLogsExport(target.toString())) {
            is AppResult.Ok -> mutableState.value = mutableState.value.copy(syncStatus = "日志已导出到 $target")
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 将当前配置导出为 JSON 写入桌面 PetalLink-config.json 文件。
     */
    fun exportConfig() {
        val target = Path.of(System.getProperty("user.home"), "Desktop", "PetalLink-config.json")
        when (val result = commands.configExportJson()) {
            is AppResult.Ok -> runCatching { java.nio.file.Files.writeString(target, result.value) }
                .onSuccess { mutableState.value = mutableState.value.copy(syncStatus = "配置已导出到 $target") }
                .onFailure { mutableState.value = mutableState.value.copy(errorMessage = it.message) }
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 弹出文件选择对话框，读取所选 JSON 并导入为应用配置。
     */
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

    /**
     * 清理应用缓存并重置 UI 状态，提示用户重新登录。
     */
    fun clearApplicationCache() = scope.launch {
        when (val result = commands.platformClearCache()) {
            is AppResult.Ok -> mutableState.value = DesktopUiState(initialized = true, syncStatus = "缓存已清理，请重新登录")
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 手动检查应用更新（不受节流限制）。
     */
    fun checkForUpdate() = scope.launch { checkForUpdateInternal(automatic = false, minimumIntervalMs = 0L) }

    /**
     * 窗口获焦触发静默检查；首次启动检查完成前忽略，之后按 10 分钟节流。
     */
    fun onWindowFocused() {
        if (!automaticUpdateChecksStarted.get()) return
        scope.launch { checkForUpdateInternal(automatic = true, minimumIntervalMs = FOCUS_UPDATE_INTERVAL_MS) }
    }

    /**
     * 更新检查内部实现：带互斥锁与最小间隔节流，区分自动/手动更新 UI 反馈。
     */
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

    /**
     * 下载并安装已发现的应用更新，完成后标记准备重启。
     */
    fun installUpdate() {
        val manifest = mutableState.value.availableUpdate ?: return
        scope.launch {
            mutableState.value = mutableState.value.copy(
                updateStatus = "等待传输并下载更新…",
                updatePhase = UpdaterPhase.DOWNLOADING,
                updateDownloadProgress = 0f,
                errorMessage = null,
            )
            when (val result = commands.updaterDownloadAndInstall(manifest) { done, total ->
                val progress = total?.takeIf { it > 0L }
                    ?.let { (done.toDouble() / it.toDouble()).coerceIn(0.0, 1.0).toFloat() }
                    ?: 0f
                mutableState.value = mutableState.value.copy(updateDownloadProgress = progress)
            }) {
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

    /**
     * 关闭更新弹窗（回到 IDLE，但保留 availableUpdate 以便侧边栏提示）。
     */
    fun dismissUpdateDialog() {
        mutableState.value = mutableState.value.copy(updatePhase = UpdaterPhase.IDLE)
    }

    /**
     * 登出当前账号并重置 UI 到等待登录状态。
     */
    fun logout() = scope.launch {
        when (val result = commands.authLogout()) {
            is AppResult.Ok -> mutableState.value = DesktopUiState(initialized = true, syncStatus = "等待登录")
            is AppResult.Err -> mutableState.value = mutableState.value.copy(errorMessage = result.error.message)
        }
    }

    /**
     * 加载并展示当前登录用户信息。
     */
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

    /**
     * 基于当前面包屑计算文件相对于挂载根的路径。
     */
    private fun relativePathFor(file: DriveFile): String =
        (browser.state.value.breadcrumbs.drop(1).map { it.name } + file.displayName()).joinToString("/")

    /**
     * 计算文件在本地挂载目录中的绝对路径，并校验不越出挂载根。
     */
    private fun localPathFor(file: DriveFile): Path {
        val root = io.github.yuanbaobaoo.petallink.config.JvmMountPaths.resolve(mutableState.value.config.mountDir)
        return root.resolve(relativePathFor(file)).normalize().also {
            require(it.startsWith(root)) { "文件路径越界" }
        }
    }

    /**
     * 取消正在进行的登录请求。
     */
    fun cancelLogin() {
        if (!mutableState.value.loggingIn) return
        scope.launch {
            commands.authCancelLogin()
            mutableState.value = mutableState.value.copy(loggingIn = false, errorMessage = null)
        }
    }

    /**
     * 设置开机自启动开关，成功则同步更新 UI 状态。
     */
    fun setLaunchAtLogin(enabled: Boolean): Boolean {
        val changed = commands.platformLaunchAtLoginSetEnabled(enabled)
        if (changed) mutableState.value = mutableState.value.copy(launchAtLogin = enabled)
        return changed
    }

    /**
     * 切换到设置页。
     */
    fun openSettings() {
        mutableState.value = mutableState.value.copy(page = AppPage.SETTINGS)
    }

    /**
     * 切换到文件页。
     */
    fun openFiles() {
        mutableState.value = mutableState.value.copy(page = AppPage.FILES)
    }

    /**
     * 校验并保存配置，成功后进入文件页；校验或保存失败时返回错误信息列表。
     */
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

    /**
     * 将字节数格式化为带合适单位（B/KB/MB/GB）的字符串。
     */
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

/**
 * 应用级 Composition Root，负责长生命周期对象的创建与逆序关闭。
 */
class ApplicationRoot(val paths: AppPaths = AppPaths.fromEnvironment()) : AutoCloseable {
    private val closed = AtomicBoolean(false)
    private val logger = Logger()
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val netGuard = NetGuard(scope)
    val commands = CommandService.create(paths, netGuard)
    val viewModel = DesktopAppViewModel(scope, commands, netGuard)

    init {
        logger.info("app.root") { "应用服务已装配，dataDir=${paths.dataDir}" }
        netGuard.startProbe()
    }

    /**
     * 逆序关闭所有长生命周期服务：停止探测、取消协程作用域并关闭命令服务。
     */
    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        netGuard.stopProbe()
        scope.cancel("application closing")
        commands.close()
        logger.info("app.root") { "应用服务已关闭" }
    }
}
