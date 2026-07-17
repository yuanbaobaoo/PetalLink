package io.github.yuanbaobaao.petallink.commands

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.auth.*
import io.github.yuanbaobaao.petallink.config.*
import io.github.yuanbaobaao.petallink.core.*
import io.github.yuanbaobaao.petallink.data.*
import io.github.yuanbaobaao.petallink.data.repository.*
import io.github.yuanbaobaao.petallink.drive.*
import io.github.yuanbaobaao.petallink.sync.*
import io.github.yuanbaobaao.petallink.sync.engine.*
import io.ktor.client.*
import io.ktor.client.engine.cio.*
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import io.github.yuanbaobaao.petallink.mount.JvmPlaceholderManager
import io.github.yuanbaobaao.petallink.mount.JvmUploadStabilityProbe
import io.github.yuanbaobaao.petallink.platform.LaunchAgentManager
import io.github.yuanbaobaao.petallink.update.JvmUpdateService
import io.github.yuanbaobaao.petallink.update.UpdateManifest

/**
 * 命令服务实现（49 个命令，对标原项目 commands/ 9 个文件）。
 *
 * 每个命令方法直接调用底层 service 对象。
 * 创建时自动构建整个 service 链（AuthService→DriveClient→FilesApi 等）。
 */
class CommandService private constructor(
    private val configStore: ConfigStore,
    private val db: PetalLinkDb,
    private val httpClient: HttpClient,
    private val tokenStore: FileTokenStore,
    private val authService: AuthService,
    private val userInfoApi: UserInfoApi,
    private val filesApi: FilesApi,
    private val changesApi: ChangesApi,
    private val downloadApi: DownloadApi,
    private val uploadApi: UploadApi,
    private val thumbnailApi: ThumbnailApi,
    private val aboutApi: AboutApi,
    private val driveClient: DriveClient,
    private val tokenRefresher: TokenRefresher,
    private val statusAggregator: StatusAggregator,
    private val envLoader: EnvLoader,
    private val syncPlan: SyncCommandPlan?,
    private val paths: AppPaths,
    private val updateService: JvmUpdateService,
) {
    @Volatile private var activeOauthServer: OauthServer? = null
    val syncStates: kotlinx.coroutines.flow.StateFlow<SyncStatusSnapshot> get() = statusAggregator.snapshots
    val folderSyncProgress: kotlinx.coroutines.flow.StateFlow<FolderSyncProgress?>? get() = syncPlan?.folderSyncProgress()
    val uploadFailures: kotlinx.coroutines.flow.SharedFlow<UploadFailedEvent>? get() = syncPlan?.uploadFailures()

    // ============ auth (7) ============
    fun authCheckSecret(): Boolean = envLoader.clientIdConfigured() && envLoader.clientSecretConfigured()

    suspend fun authRestore(): AppResult<AuthState> {
        return try {
            val stored = tokenStore.load()
            if (stored != null) {
                authService.ensureValidAccessToken()
                val config = configStore.load() ?: UserConfig()
                if (config.mountConfigured && config.mountDir.isNotBlank()) syncPlan?.start()
            }
            AppResult.Ok(AuthState(
                loggedIn = tokenStore.load() != null,
                secretConfigured = authCheckSecret(),
                callbackPort = (configStore.load() ?: UserConfig()).oauthCallbackPort,
            ))
        } catch (error: AppError) {
            AppResult.Err(error)
        } catch (error: Throwable) {
            AppResult.Err(AppError.Auth(error.message ?: "恢复登录状态失败"))
        }
    }

    suspend fun authLogin(port: Int): AppResult<TokenPair> {
        return try {
            val redirectUri = Pkce.buildRedirectUri(port)
            val pkce = Pkce.generate()
            val expectedState = Pkce.generateState()
            val oauth = OauthServer(port)
            check(activeOauthServer == null) { "已有登录流程正在进行" }
            activeOauthServer = oauth
            try {
                oauth.bind()
                val authorizeUrl = Pkce.buildAuthorizeUrl(
                    redirectUri, expectedState, pkce, envLoader.resolvedClientId(),
                )
                ProcessBuilder("open", authorizeUrl).start()
                val result = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                    oauth.waitForCallback()
                }
                val code = OauthCallbackValidator.requireCode(result, expectedState)
                val token = authService.exchangeCodeForToken(code, redirectUri, pkce.codeVerifier)
                resetAccountRuntimeAndMount()
                AppResult.Ok(token)
            } finally {
                oauth.stop()
                activeOauthServer = null
            }
        } catch (e: AppError) { AppResult.Err(e) }
        catch (e: Throwable) { AppResult.Err(AppError.Auth(e.message ?: "auth error")) }
    }

    suspend fun authCancelLogin(): AppResult<Unit> = safe {
        activeOauthServer?.stop()
        activeOauthServer = null
    }

    suspend fun authLogout(): AppResult<Unit> {
        return try {
            resetAccountRuntimeAndMount()
            tokenStore.clear()
            AppResult.Ok(Unit)
        } catch (error: Throwable) {
            AppResult.Err(AppError.Internal(error.message ?: "登出清理失败"))
        }
    }

    suspend fun authGetUserInfo(): AppResult<UserInfo> = try {
        AppResult.Ok(userInfoApi.get())
    } catch (e: Throwable) {
        AppResult.Err(AppError.Auth(e.message ?: "auth error"))
    }

    suspend fun authIsLoggedIn(): AppResult<Boolean> = safe {
        val token = tokenStore.loadSuspended()
        token != null && !token.isExpired(System.currentTimeMillis())
    }

    // ============ config (4) ============
    fun configLoad(): AppResult<UserConfig> = safe { configStore.load() ?: UserConfig() }
    fun configSave(config: UserConfig): AppResult<Unit> {
        val previous = try {
            configStore.load() ?: UserConfig()
        } catch (error: Throwable) {
            return AppResult.Err(AppError.Internal(error.message ?: "读取旧配置失败"))
        }
        syncPlan?.prepareConfigurationChange()
        return try {
            configStore.save(config)
            if (config.mountConfigured && config.mountDir.isNotBlank() && tokenStore.loadSuspended() != null) {
                syncPlan?.start()
            }
            syncPlan?.configurationChanged(previous, config)
            AppResult.Ok(Unit)
        } catch (error: Throwable) {
            syncPlan?.configurationChangeFailed()
            AppResult.Err(AppError.Internal(error.message ?: "保存配置失败"))
        }
    }
    fun configExportJson(): AppResult<String> = safe { Json.encodeToString(UserConfig.serializer(), configStore.load() ?: UserConfig()) }
    fun configImportJson(jsonStr: String): AppResult<UserConfig> {
        val config = try {
            (configStore as? JsonConfigStore)?.parseImport(jsonStr)
                ?: Json { ignoreUnknownKeys = true }.decodeFromString(UserConfig.serializer(), jsonStr)
        } catch (error: Throwable) {
            return AppResult.Err(AppError.Internal(error.message ?: "配置解析失败"))
        }
        ConfigValidator.validate(config).firstOrNull()?.let {
            return AppResult.Err(AppError.Internal(it))
        }
        return when (val saved = configSave(config)) {
            is AppResult.Ok -> AppResult.Ok(config)
            is AppResult.Err -> saved
        }
    }

    // ============ drive (12) ============
    suspend fun driveList(parentId: String?, cursor: String?, pageSize: Int?): AppResult<FileListResult> = drive {
        val (files, next) = filesApi.listFiles(parentId, pageSize ?: 100, cursor)
        FileListResult(files, next)
    }
    suspend fun driveListAll(parentId: String?): AppResult<List<DriveFile>> = drive { filesApi.listAllFiles(parentId) }
    suspend fun driveGetFile(id: String): AppResult<DriveFile> = drive { filesApi.getFile(id) }
    suspend fun driveCreateFolder(name: String, parentId: String?): AppResult<DriveFile> = drive {
        val file = exclusiveSyncMutation { filesApi.createFile(name, parentId, true) }
        syncPlan?.remoteMutationCommitted()
        file
    }
    suspend fun driveDeleteFile(id: String, name: String?): AppResult<Unit> = drive {
        var remoteCommitted = false
        try {
            exclusiveSyncMutation {
                val settler = JvmDriveMutationSettler(configStore, db)
                val plan = settler.planDelete(id)
                filesApi.deleteFile(id)
                remoteCommitted = true
                settler.settleDelete(plan, name)
            }
        } finally {
            if (remoteCommitted) syncPlan?.remoteMutationCommitted()
        }
    }
    suspend fun driveRenameFile(id: String, newName: String): AppResult<DriveFile> = drive {
        var remoteCommitted = false
        try {
            exclusiveSyncMutation {
                val settler = JvmDriveMutationSettler(configStore, db)
                val plan = settler.planRename(id, newName)
                val file = filesApi.updateFile(id, newName)
                remoteCommitted = true
                if (plan != null) settler.settlePathChange(plan, file)
                file
            }
        } finally {
            if (remoteCommitted) syncPlan?.remoteMutationCommitted()
        }
    }
    suspend fun driveMoveFile(id: String, newParentFolder: String): AppResult<DriveFile> =
        drive {
            var remoteCommitted = false
            try {
                exclusiveSyncMutation {
                    val settler = JvmDriveMutationSettler(configStore, db)
                    val plan = settler.planMove(id, newParentFolder)
                    val current = filesApi.getFile(id)
                    val oldParent = DriveParsers.singleParent(current, "move preflight")
                    val file = filesApi.moveFile(id, oldParent, newParentFolder)
                    remoteCommitted = true
                    if (plan != null) settler.settlePathChange(plan, file)
                    file
                }
            } finally {
                if (remoteCommitted) syncPlan?.remoteMutationCommitted()
            }
        }
    suspend fun driveSearch(keyword: String, parentId: String?, pageSize: Int?): AppResult<FileListResult> = drive {
        val (files, next) = filesApi.search(keyword, parentId, pageSize ?: 100)
        FileListResult(files, next)
    }
    suspend fun driveGetThumbnail(fileId: String): AppResult<ByteArray> = drive { thumbnailApi.getThumbnail(fileId) }
    suspend fun driveGetAbout(): AppResult<DriveQuota> = drive { aboutApi.getQuota() }
    suspend fun driveDownloadFile(fileId: String, destPath: String): AppResult<Unit> = drive {
        exclusiveSyncMutation { runDownloadCommand(fileId, destPath) }
        Unit
    }
    suspend fun driveUploadFile(localPath: String, parentId: String?): AppResult<DriveFile> = drive {
        val file = exclusiveSyncMutation { runUploadCommand(localPath, parentId) }
        syncPlan?.remoteMutationCommitted()
        file
    }

    // ============ sync_control (2) ============
    suspend fun syncManualRefresh(): AppResult<Unit> = syncPlan?.manualRefresh() ?: AppResult.Err(AppError.Internal("engine not started"))
    suspend fun syncRetryFailed(): AppResult<Unit> = syncPlan?.retryFailed() ?: AppResult.Err(AppError.Internal("engine not started"))
    fun syncNetworkRecovered() { syncPlan?.networkRecovered() }

    // ============ sync_status (4) ============
    suspend fun syncState(): AppResult<SyncStatusSnapshot> = dbSafeSusp {
        statusAggregator.snapshot(db, statusAggregator.snapshots.value.runtime)
    }
    suspend fun syncSnapshot(): AppResult<SyncStatusSnapshot> = safe { statusAggregator.snapshots.value }
    suspend fun syncItemsByFolder(folderLocalPath: String): AppResult<List<SyncItem>> = dbSafeSusp {
        val normalized = folderLocalPath.trim('/').takeIf(String::isNotBlank)
        db.syncItems.selectByFolderPrefix(normalized?.let { "$it/" }.orEmpty())
    }
    suspend fun syncCheckFileLocalStatus(fileId: String): AppResult<String> = dbSafeSusp {
        JvmSyncStatusResolver(configStore, db.syncItems).resolveOne(fileId)
    }
    suspend fun syncBatchFileStatus(fileIds: List<String>): AppResult<Map<String, String>> = dbSafeSusp {
        JvmSyncStatusResolver(configStore, db.syncItems).resolveBatch(fileIds)
    }

    // ============ transfer (6) ============
    suspend fun transferListAll(): AppResult<List<TransferTask>> = dbSafeSusp {
        db.transfers.selectAll()
    }
    fun transferHasActive(): Boolean = try {
        runBlocking {
            listOf(
                TransferState.Pending, TransferState.Running, TransferState.WaitingForNetwork,
                TransferState.BackingOff, TransferState.VerifyingRemote, TransferState.RestartRequired,
            ).any { db.transfers.selectByState(it).isNotEmpty() }
        }
    } catch (e: Throwable) { false }
    suspend fun transferClearCompleted(): AppResult<Unit> = clearTransferHistory(true, false)
    suspend fun transferClearFailed(): AppResult<Unit> = clearTransferHistory(false, true)
    suspend fun transferClearFinished(): AppResult<Unit> = clearTransferHistory(true, true)
    suspend fun transferRetry(taskId: Long): AppResult<Unit> = dbSafeSusp {
        when (val result = syncPlan?.retryTransfer(taskId)) {
            is AppResult.Err -> throw result.error
            else -> Unit
        }
    }

    // ============ folder_sync (1) ============
    suspend fun syncFolderRecursive(folderId: String, relPath: String): AppResult<Long> = drive {
        val accepted = syncPlan?.enqueueFolderSync(folderId, relPath)
            ?: throw AppError.Internal("同步引擎未启动")
        if (!accepted) throw AppError.Internal("已有同步周期或目录同步正在运行，本次请求未开始")
        0L
    }

    // ============ free_up (5) ============
    suspend fun syncCheckSafeFreeUp(relPath: String, fileId: String): AppResult<String> = drive {
        freeUpService().checkSafe(relPath, fileId)
    }
    suspend fun syncListFreeableInFolder(folderRelPath: String): AppResult<List<FreeableItem>> = dbSafeSusp {
        // 查 DB sync_items 中该目录下所有已同步且有文件大小的项（可释放空间）
        val prefix = if (folderRelPath.isEmpty() || folderRelPath == "/") "" else "$folderRelPath/"
        val items = db.syncItems.selectByFolderPrefix(prefix)
        val root = configuredMountRoot()
        items.filter { it.syncStatus == SyncStatus.SYNCED && it.localSize != null && it.localSize >= 0 && !it.isFolder }.map { sync ->
            FreeableItem(
                fileId = sync.fileId,
                relPath = sync.localPath,
                localPath = root.resolve(sync.localPath).normalize().toString(),
                name = sync.localPath.substringAfterLast("/"),
                size = sync.localSize!!,
            )
        }
    }
    suspend fun syncFreeUpSpace(fileId: String, relPath: String, localPath: String, name: String, size: Long): AppResult<Unit> = drive {
        val expected = configuredMountRoot().resolve(relPath).normalize()
        if (Paths.get(localPath).toAbsolutePath().normalize() != expected) {
            throw AppError.LocalIo("释放空间路径与 relPath 不一致")
        }
        freeUpService().freeOne(relPath, fileId, size)
        Unit
    }
    suspend fun syncFreeUpBatch(items: List<FreeableItem>): AppResult<FreeUpBatchResult> = drive {
        var ok = 0; var fail = 0; var freedBytes = 0L; val errs = mutableListOf<String>()
        for (item in items) {
            try {
                freedBytes += freeUpService().freeOne(item.relPath, item.fileId, item.size)
                ok++
            }
            catch (e: Throwable) { fail++; errs.add("${item.name}: ${e.message}") }
        }
        FreeUpBatchResult(ok, fail, freedBytes, errs)
    }
    suspend fun syncDownloadOnDemand(fileId: String, destPath: String): AppResult<Boolean> = drive {
        val (root, relative, destination) = resolveCommandPath(destPath)
        val baseline = db.syncItems.findByFileId(fileId)
        if (baseline != null && baseline.localPath != relative) {
            throw AppError.LocalIo("下载路径与 fileId 同步基线不一致")
        }
        exclusiveSyncMutation { runDownloadCommand(fileId, destination.toString()) }
        JvmPlaceholderManager(root).markDownloaded(destination.toString())
        true
    }

    // ============ platform (8) ============
    suspend fun platformOpenInFinder(path: String): AppResult<Boolean> = safe {
        ProcessBuilder("open", path).start()
        true
    }
    fun platformLaunchAtLoginIsEnabled(): Boolean = runCatching { launchAgentManager().isEnabled() }.getOrDefault(false)
    fun platformLaunchAtLoginSetEnabled(enabled: Boolean): Boolean = runCatching {
        launchAgentManager().setEnabled(enabled)
        launchAgentManager().isEnabled() == enabled
    }.getOrDefault(false)
    suspend fun platformClearCache(): AppResult<Unit> = safe {
        syncPlan?.stop()
        runBlocking { tokenStore.clear() }
        runBlocking { db.clearAll() }
        Files.deleteIfExists(paths.configFile)
        if (Files.exists(paths.dataDir)) Files.list(paths.dataDir).use { files ->
            files.filter {
                val name = it.fileName.toString()
                name.startsWith("syncstate_") || name.startsWith("cloudtree_") ||
                    name.startsWith("changes_cursor_") || name == "incomplete-shutdown"
            }.forEach(Files::deleteIfExists)
        }
    }
    fun platformLogsList(): AppResult<List<io.github.yuanbaobaao.petallink.core.logging.LogRecord>> = safe {
        val logger = io.github.yuanbaobaao.petallink.core.logging.Logger()
        logger.snapshot(1000)
    }
    fun platformLogsExport(path: String): AppResult<Unit> = safe {
        io.github.yuanbaobaao.petallink.core.logging.LoggerRuntime.exportTo(Paths.get(path))
    }
    fun platformLogsClear(): AppResult<Unit> = safe {
        io.github.yuanbaobaao.petallink.core.logging.LoggerRuntime.clear()
    }
    fun platformAppGetVersion(): String = BuildInfo.VERSION
    suspend fun updaterCheck(): AppResult<UpdateManifest?> = drive { updateService.check() }
    suspend fun updaterDownloadAndInstall(manifest: UpdateManifest): AppResult<Boolean> = drive {
        val staged = updateService.downloadAndStage(manifest, ::transferHasActive)
        updateService.launchInstaller(staged)
    }

    fun close() {
        runBlocking { syncPlan?.closeGracefully() }
        httpClient.close()
        db.close()
    }

    private fun configuredMountRoot(): Path {
        val config = configStore.load() ?: throw AppError.LocalIo("尚未配置挂载目录")
        if (!config.mountConfigured || config.mountDir.isBlank()) throw AppError.LocalIo("尚未配置挂载目录")
        return JvmMountPaths.resolve(config.mountDir)
    }

    private suspend fun resetAccountRuntimeAndMount() {
        syncPlan?.stop()
        db.clearAll()
        val current = configStore.load() ?: UserConfig()
        configStore.save(current.copy(mountDir = "", mountConfigured = false))
        clearSyncCacheFiles()
    }

    private fun clearSyncCacheFiles() {
        if (!Files.exists(paths.dataDir)) return
        Files.list(paths.dataDir).use { files ->
            files.filter { path ->
                val name = path.fileName.toString()
                name.startsWith("syncstate_") || name.startsWith("cloudtree_") ||
                    name.startsWith("changes_cursor_")
            }.forEach(Files::deleteIfExists)
        }
    }

    private fun freeUpService(): JvmFreeUpService {
        val root = configuredMountRoot()
        return JvmFreeUpService(
            root,
            paths,
            db,
            JvmPlaceholderManager(root),
            FilesApiFreeUpVerifier(filesApi),
        )
    }

    private fun launchAgentManager(): LaunchAgentManager {
        val command = ProcessHandle.current().info().command().orElseGet {
            Paths.get(System.getProperty("java.home"), "bin", "java").toString()
        }
        return LaunchAgentManager(AppPaths.PROD_BUNDLE_ID, Paths.get(command))
    }

    private suspend fun clearTransferHistory(completed: Boolean, failed: Boolean): AppResult<Unit> = dbSafeSusp {
        db.transfers.clearHistory(completed, failed)
        statusAggregator.snapshot(db, statusAggregator.snapshots.value.runtime)
        Unit
    }

    private suspend fun <T> exclusiveSyncMutation(block: suspend () -> T): T =
        syncPlan?.exclusiveMutation(block) ?: block()

    private suspend fun runUploadCommand(localPath: String, parentId: String?): DriveFile {
        val store = JvmTransferFileStore()
        val (_, relative, source) = resolveCommandPath(localPath)
        val snapshot = store.snapshot(source.toString())
        val id = db.transfers.insert(
            TransferTask(
                id = null,
                direction = TransferDirection.UPLOAD,
                fileId = null,
                localPath = source.toString(),
                name = source.fileName.toString(),
                totalSize = snapshot.size,
                state = TransferState.Pending,
                errorMessage = null,
                createdAt = System.currentTimeMillis(),
                relativePath = relative,
                parentFileId = parentId,
                operation = 0,
                sourceMtime = snapshot.modifiedAtMillis,
                sourceSize = snapshot.size,
            ),
        )
        val disposition = commandTaskRunner(store).runExpected(commandTaskContext(db.transfers.findById(id)!!))
        if (disposition != TaskDisposition.COMPLETED) {
            val task = db.transfers.findById(id)
            throw AppError.Internal(task?.errorMessage ?: "上传未完成: $disposition")
        }
        val remoteId = db.transfers.findById(id)?.remoteResultFileId
            ?: throw AppError.Data("上传完成但缺少 remote_result_file_id")
        return filesApi.getFile(remoteId)
    }

    private suspend fun runDownloadCommand(fileId: String, destPath: String) {
        val store = JvmTransferFileStore()
        val (_, relative, destination) = resolveCommandPath(destPath)
        val metadata = downloadApi.fetchRemoteMetadata(fileId)
        val isUpdate = Files.isRegularFile(destination, java.nio.file.LinkOption.NOFOLLOW_LINKS) && Files.size(destination) > 0
        val id = db.transfers.insert(
            TransferTask(
                id = null,
                direction = if (isUpdate) TransferDirection.DOWNLOAD_UPDATE else TransferDirection.DOWNLOAD,
                fileId = fileId,
                localPath = destination.toString(),
                name = destination.fileName.toString(),
                totalSize = metadata.size,
                state = TransferState.Pending,
                errorMessage = null,
                createdAt = System.currentTimeMillis(),
                relativePath = relative,
                operation = if (isUpdate) 3 else 2,
                expectedCloudEditedTime = metadata.editedTime?.let { java.time.Instant.parse(it).toEpochMilli() },
            ),
        )
        val disposition = commandTaskRunner(store).runExpected(commandTaskContext(db.transfers.findById(id)!!))
        if (disposition != TaskDisposition.COMPLETED) {
            val task = db.transfers.findById(id)
            throw AppError.Internal(task?.errorMessage ?: "下载未完成: $disposition")
        }
    }

    private fun resolveCommandPath(raw: String): Triple<Path, String, Path> {
        val root = configuredMountRoot().toRealPath()
        val requested = Paths.get(raw).toAbsolutePath().normalize()
        if (!requested.startsWith(root) || requested == root) throw AppError.LocalIo("路径不在挂载目录内: $raw")
        val relativePath = root.relativize(requested)
        if (relativePath.none() || relativePath.any { it.toString() == "." || it.toString() == ".." }) {
            throw AppError.LocalIo("非法挂载相对路径: $raw")
        }
        var current = root
        for (segment in relativePath) {
            current = current.resolve(segment)
            if (Files.exists(current, java.nio.file.LinkOption.NOFOLLOW_LINKS) && Files.isSymbolicLink(current)) {
                throw AppError.LocalIo("拒绝操作符号链接: $current")
            }
        }
        return Triple(root, relativePath.joinToString("/"), requested)
    }

    private fun commandTaskRunner(store: JvmTransferFileStore): TaskRunner {
        val probe = JvmUploadStabilityProbe()
        val operations = TransferOperationsImpl(
            uploadApi = uploadApi,
            downloadApi = downloadApi,
            readFileBytes = { Files.readAllBytes(Paths.get(it)) },
            writeFileBytes = { path, bytes -> Files.write(Paths.get(path), bytes) },
            fileExists = store::exists,
            fileSize = store::size,
            uploadStability = probe,
            fileStore = store,
            remoteVerification = JvmRemoteTransferVerifier(filesApi, store)::verify,
            deleteRemote = filesApi::deleteFile,
        )
        return TaskRunner(db.transfers, operations, { true }, System::currentTimeMillis)
    }

    private fun commandTaskContext(task: TransferTask) = TaskContext(
        id = task.id ?: error("传输任务缺少 id"),
        fileId = task.fileId.orEmpty(),
        localPath = task.localPath.orEmpty(),
        direction = task.direction,
        state = task.state,
        stateRevision = task.stateRevision,
        attempt = task.attempt,
        bytesTotal = task.bytesTotal,
        bytesDone = task.bytesDone,
        nextRetryAt = task.nextRetryAt,
        remoteResultFileId = task.remoteResultFileId,
        sessionUrl = task.sessionUrl,
        serverId = task.serverId,
        uploadId = task.uploadId,
        parentFileId = task.parentFileId,
        operation = task.operation,
        sourceMtime = task.sourceMtime,
        sourceSize = task.sourceSize,
        expectedCloudEditedTime = task.expectedCloudEditedTime,
        createdAt = task.createdAt,
    )

    // ============ helpers ============
    private fun <T> safe(block: () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Internal(e.message ?: "unknown")) }
    private suspend fun <T> drive(block: suspend () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: AppError) { AppResult.Err(e) } catch (e: Throwable) { AppResult.Err(AppError.Remote(0, e.message ?: "drive error")) }
    private fun <T> dbSafe(block: () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Data(e.message ?: "db error")) }
    private suspend fun <T> dbSafeSusp(block: suspend () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Data(e.message ?: "db error")) }

    companion object {
        /**
         * 工厂方法：创建完整的 CommandService 并自动布线 service 链。
         */
        fun create(paths: AppPaths = AppPaths.fromEnvironment()): CommandService {
            io.github.yuanbaobaao.petallink.core.logging.LoggerRuntime.configure(paths.logsDir)
            val httpClient = HttpClient(CIO) {
                engine { requestTimeout = 60_000; endpoint.connectTimeout = 15_000 }
            }
            val envLoader = EnvLoader.apply { loadEnvFile() }
            val configStore = JsonConfigStore(paths.configFile)
            val db = PetalLinkDb(paths.databaseFile.toString())
            val statusAgg = StatusAggregator()

            val tokenStore = FileTokenStore(paths.tokenFile)
            val tokenRefresher = TokenRefresher(
                httpClient, envLoader::resolvedClientId, envLoader::resolvedClientSecret,
                { runBlocking { tokenStore.load() } }, { runBlocking { tokenStore.save(it) } },
            )
            val authService = AuthService(
                httpClient, envLoader::resolvedClientId, envLoader::resolvedClientSecret,
                tokenStore, tokenRefresher,
            )
            val userInfoApi = UserInfoApi(httpClient, authService::ensureValidAccessToken)
            val provider = suspend { authService.ensureValidAccessToken() }
            val driveClient = DriveClient(httpClient, provider, { runBlocking { tokenRefresher.refresh() } })
            val filesApi = FilesApi(driveClient)
            val changesApi = ChangesApi(driveClient)
            val downloadApi = DownloadApi(driveClient)
            val uploadApi = UploadApi(driveClient)
            val thumbnailApi = ThumbnailApi(driveClient)
            val aboutApi = AboutApi(driveClient)
            val syncPlan = JvmSyncRuntime(
                paths, configStore, db, filesApi, changesApi, uploadApi, downloadApi, statusAgg,
            )
            val updateService = JvmUpdateService(
                httpClient, paths, BuildInfo.VERSION, BuildInfo.UPDATE_ENDPOINT, BuildInfo.UPDATE_TEAM_ID,
            )

            return CommandService(
                configStore, db, httpClient, tokenStore, authService, userInfoApi,
                filesApi, changesApi, downloadApi, uploadApi, thumbnailApi, aboutApi,
                driveClient, tokenRefresher, statusAgg, envLoader, syncPlan, paths, updateService,
            )
        }
    }
}

/**
 * 文件 TokenStore 实现（token.bin 在 Application Support 目录）。
 * 用 ChaCha20-Poly1305 加密（对标原项目 token_store.rs）。
 *
 * token.bin 字节布局：
 * - [0..4]   MAGIC = "PTL1"（4 字节）
 * - [4..16]  nonce（12 字节）
 * - [16..]   ciphertext + 16B Poly1305 tag
 * key = SHA-256(IOPlatformUUID)，无 salt（机器绑定）。
 */
class FileTokenStore(tokenPath: Path = AppPaths.fromEnvironment().tokenFile) : TokenStore {
    private val file = tokenPath.toFile()
    private val json = Json { ignoreUnknownKeys = true }
    private val MAGIC = byteArrayOf('P'.code.toByte(), 'T'.code.toByte(), 'L'.code.toByte(), '1'.code.toByte())
    private val nonceLen = 12

    /** 用 IOPlatformUUID 派生加密密钥（对标原项目 machine-bound） */
    private fun deriveKey(): ByteArray {
        val uuid = readMachineUUID()
            ?: throw AppError.Auth("无法读取 IOPlatformUUID，拒绝使用不安全的降级密钥")
        val digest = java.security.MessageDigest.getInstance("SHA-256")
        return digest.digest(uuid.toByteArray())
    }

    private fun readMachineUUID(): String? {
        return try {
            // 用 ioreg 读取 IOPlatformUUID（对标原项目）
            val proc = ProcessBuilder("ioreg", "-d2", "-c", "IOPlatformExpertDevice")
                .redirectErrorStream(true).start()
            val output = proc.inputStream.bufferedReader().readText()
            val match = Regex("\"IOPlatformUUID\"\\s*=\\s*\"([^\"]+)\"").find(output)
            match?.groupValues?.get(1)?.takeIf { it.isNotBlank() }
        } catch (e: Throwable) {
            null
        }
    }

    override suspend fun load(): TokenPair? {
        if (!file.exists()) return null
        return try {
            val data = file.readBytes()
            if (data.size < MAGIC.size + nonceLen + 16) return null
            // 验证 MAGIC
            if (!data.take(MAGIC.size).toByteArray().contentEquals(MAGIC)) return null
            val nonce = data.copyOfRange(MAGIC.size, MAGIC.size + nonceLen)
            val ciphertext = data.copyOfRange(MAGIC.size + nonceLen, data.size)
            val key = deriveKey()
            val plaintext = io.github.yuanbaobaao.petallink.platform.ChaCha20Poly1305.decrypt(key, nonce, ciphertext)
            val tokenPair = TokenSerializer.deserialize(plaintext)
            tokenPair
        } catch (e: Throwable) { null }
    }

    override suspend fun save(token: TokenPair) {
        val path = file.toPath()
        Files.createDirectories(path.parent)
        val plaintext = TokenSerializer.serialize(token)
        val key = deriveKey()
        // 随机 nonce（每次保存重新生成）
        val nonce = randomBytes(nonceLen)
        val ciphertext = io.github.yuanbaobaao.petallink.platform.ChaCha20Poly1305.encrypt(key, nonce, plaintext)
        val data = MAGIC + nonce + ciphertext
        val temp = Files.createTempFile(path.parent, "token-", ".tmp")
        try {
            Files.write(temp, data)
            Files.setPosixFilePermissions(
                temp,
                java.nio.file.attribute.PosixFilePermissions.fromString("rw-------"),
            )
            try {
                Files.move(
                    temp, path,
                    java.nio.file.StandardCopyOption.ATOMIC_MOVE,
                    java.nio.file.StandardCopyOption.REPLACE_EXISTING,
                )
            } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
                Files.move(temp, path, java.nio.file.StandardCopyOption.REPLACE_EXISTING)
            }
            Files.setPosixFilePermissions(
                path,
                java.nio.file.attribute.PosixFilePermissions.fromString("rw-------"),
            )
        } finally {
            Files.deleteIfExists(temp)
        }
    }

    override suspend fun clear() { file.delete() }

    fun loadSuspended(): TokenPair? = runBlocking { load() }
    private fun randomBytes(n: Int) = java.security.SecureRandom().run { ByteArray(n).also { nextBytes(it) } }

}

// ============ AppResult 类型 ============
sealed class AppResult<out T> {
    data class Ok<T>(val value: T) : AppResult<T>()
    data class Err(val error: AppError) : AppResult<Nothing>()
}

data class AuthState(val loggedIn: Boolean, val secretConfigured: Boolean, val callbackPort: Int)
data class FileListResult(val files: List<io.github.yuanbaobaao.petallink.drive.DriveFile>, val nextCursor: String?)
data class FreeableItem(val fileId: String, val relPath: String, val localPath: String, val name: String, val size: Long)
data class FreeUpBatchResult(
    val freedCount: Int,
    val skippedCount: Int,
    val freedBytes: Long,
    val errors: List<String>,
)
data class FolderSyncProgress(val done: Int, val total: Int)

interface SyncCommandPlan : AutoCloseable {
    suspend fun manualRefresh(): AppResult<Unit>
    suspend fun retryFailed(): AppResult<Unit>
    suspend fun retryTransfer(taskId: Long): AppResult<Unit> = retryFailed()
    fun prepareConfigurationChange() = Unit
    fun configurationChanged(previous: UserConfig, current: UserConfig) = Unit
    fun configurationChangeFailed() = Unit
    fun start() = Unit
    fun stop() = Unit
    fun networkRecovered() = Unit
    suspend fun <T> exclusiveMutation(block: suspend () -> T): T = block()
    fun remoteMutationCommitted() = Unit
    fun enqueueFolderSync(folderId: String, relativePath: String): Boolean = false
    fun folderSyncProgress(): kotlinx.coroutines.flow.StateFlow<FolderSyncProgress?>? = null
    fun uploadFailures(): kotlinx.coroutines.flow.SharedFlow<UploadFailedEvent>? = null
    suspend fun closeGracefully(timeoutMs: Long = 3_200L): Boolean {
        close()
        return true
    }
    override fun close() = Unit
}
