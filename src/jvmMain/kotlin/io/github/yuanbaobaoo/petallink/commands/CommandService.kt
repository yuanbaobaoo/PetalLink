package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.auth.*
import io.github.yuanbaobaoo.petallink.config.*
import io.github.yuanbaobaoo.petallink.core.*
import io.github.yuanbaobaoo.petallink.data.*
import io.github.yuanbaobaoo.petallink.data.repository.*
import io.github.yuanbaobaoo.petallink.drive.*
import io.github.yuanbaobaoo.petallink.sync.*
import io.github.yuanbaobaoo.petallink.sync.engine.*
import io.ktor.client.*
import io.ktor.client.engine.cio.*
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import io.github.yuanbaobaoo.petallink.mount.JvmPlaceholderManager
import io.github.yuanbaobaoo.petallink.mount.JvmUploadStabilityProbe
import io.github.yuanbaobaoo.petallink.platform.LaunchAgentManager
import io.github.yuanbaobaoo.petallink.core.net_guard.NetGuard
import io.github.yuanbaobaoo.petallink.core.net_guard.NetState
import io.github.yuanbaobaoo.petallink.update.JvmUpdateService
import io.github.yuanbaobaoo.petallink.update.UpdateManifest

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
    private val netGuard: NetGuard?,
    private val eventHub: SyncEventHub,
) {
    @Volatile private var activeOauthServer: OauthServer? = null
    val syncStates: kotlinx.coroutines.flow.StateFlow<SyncStatusSnapshot> get() = statusAggregator.snapshots
    val folderSyncProgress: kotlinx.coroutines.flow.StateFlow<FolderSyncProgress?>? get() = syncPlan?.folderSyncProgress()
    val uploadFailures: kotlinx.coroutines.flow.SharedFlow<UploadFailedEvent>? get() = syncPlan?.uploadFailures()
    val transferUpdates: kotlinx.coroutines.flow.SharedFlow<TransferUpdateEvent> get() = eventHub.transferUpdates

    // ============ auth (7) ============
    /**
     * 检查 OAuth clientId / clientSecret 是否已配置。
     */
    fun authCheckSecret(): Boolean = envLoader.clientIdConfigured() && envLoader.clientSecretConfigured()

    /**
     * 从磁盘恢复登录状态；若已登录且挂载目录已配置则启动同步引擎。
     */
    suspend fun authRestore(): AppResult<AuthState> {
        return try {
            val stored = tokenStore.load()
            if (stored != null) {
                authService.ensureValidAccessToken()
                val config = configStore.load() ?: UserConfig()
                if (config.mountConfigured && config.mountDir.isNotBlank()) syncPlan?.start()
            } else {
                resetAccountRuntimeAndMount()
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

    /**
     * 启动本地 OAuth 回调服务、打开浏览器发起授权，并完成 code 换 token。
     */
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

    /**
     * 取消正在进行的 OAuth 登录流程，停止本地回调服务。
     */
    suspend fun authCancelLogin(): AppResult<Unit> = safe {
        activeOauthServer?.stop()
        activeOauthServer = null
    }

    /**
     * 注销当前账号：重置运行时与挂载状态，并清空本地 token。
     */
    suspend fun authLogout(): AppResult<Unit> {
        return try {
            resetAccountRuntimeAndMount()
            tokenStore.clear()
            AppResult.Ok(Unit)
        } catch (error: Throwable) {
            AppResult.Err(AppError.Internal(error.message ?: "登出清理失败"))
        }
    }

    /**
     * 获取当前登录用户的信息。
     */
    suspend fun authGetUserInfo(): AppResult<UserInfo> = try {
        AppResult.Ok(userInfoApi.get())
    } catch (e: Throwable) {
        AppResult.Err(AppError.Auth(e.message ?: "auth error"))
    }

    /**
     * 判断当前是否已登录且 token 未过期。
     */
    suspend fun authIsLoggedIn(): AppResult<Boolean> = safe {
        val token = tokenStore.loadSuspended()
        token != null && !token.isExpired(System.currentTimeMillis())
    }

    // ============ config (4) ============
    /**
     * 读取用户配置；若不存在则返回默认配置。
     */
    fun configLoad(): AppResult<UserConfig> = safe { configStore.load() ?: UserConfig() }
    /**
     * 保存用户配置；在配置变更前后通知同步引擎，并按需启动同步。
     */
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
    /**
     * 将当前用户配置序列化为 JSON 字符串。
     */
    fun configExportJson(): AppResult<String> = safe { Json.encodeToString(UserConfig.serializer(), configStore.load() ?: UserConfig()) }
    /**
     * 解析并校验导入的 JSON 配置，成功后保存。
     */
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
    /**
     * 分页列出指定父目录下的云端文件。
     */
    suspend fun driveList(parentId: String?, cursor: String?, pageSize: Int?): AppResult<FileListResult> = drive {
        val (files, next) = filesApi.listFiles(parentId, pageSize ?: 100, cursor)
        FileListResult(files, next)
    }
    /**
     * 列出指定父目录下的全部云端文件（不分页）。
     */
    suspend fun driveListAll(parentId: String?): AppResult<List<DriveFile>> = drive { filesApi.listAllFiles(parentId) }
    /**
     * 按 fileId 获取单个云端文件元数据。
     */
    suspend fun driveGetFile(id: String): AppResult<DriveFile> = drive { filesApi.getFile(id) }
    /**
     * 在云端创建文件夹，并通知同步引擎远端已发生写操作。
     */
    suspend fun driveCreateFolder(name: String, parentId: String?): AppResult<DriveFile> = drive {
        val file = exclusiveSyncMutation { filesApi.createFile(name, parentId, true) }
        syncPlan?.remoteMutationCommitted()
        file
    }
    /**
     * 删除云端文件/文件夹，并在同步互斥下结算本地基线与删除留痕。
     */
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
    /**
     * 重命名云端文件，并结算本地路径与同步基线。
     */
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
    /**
     * 将云端文件移动到新的父目录，并结算本地路径与同步基线。
     */
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
    /**
     * 按关键字在指定父目录下搜索云端文件。
     */
    suspend fun driveSearch(keyword: String, parentId: String?, pageSize: Int?): AppResult<FileListResult> = drive {
        val (files, next) = filesApi.search(keyword, parentId, pageSize ?: 100)
        FileListResult(files, next)
    }
    /**
     * 获取云端文件的缩略图字节。
     */
    suspend fun driveGetThumbnail(fileId: String): AppResult<ByteArray> = drive { thumbnailApi.getThumbnail(fileId) }
    /**
     * 获取云端存储空间的配额信息。
     */
    suspend fun driveGetAbout(): AppResult<DriveQuota> = drive { aboutApi.getQuota() }
    /**
     * 下载云端文件到挂载目录内的指定路径。
     */
    suspend fun driveDownloadFile(fileId: String, destPath: String): AppResult<Unit> = drive {
        exclusiveSyncMutation { runDownloadCommand(fileId, destPath) }
        Unit
    }
    /**
     * 上传挂载目录内的本地文件到云端指定父目录。
     */
    suspend fun driveUploadFile(localPath: String, parentId: String?): AppResult<DriveFile> = drive {
        val file = exclusiveSyncMutation { runUploadCommand(localPath, parentId) }
        syncPlan?.remoteMutationCommitted()
        file
    }

    // ============ sync_control (2) ============
    /**
     * 触发一次完整的本地+云端手动刷新同步周期。
     */
    suspend fun syncManualRefresh(): AppResult<Unit> = syncPlan?.manualRefresh() ?: AppResult.Err(AppError.Internal("engine not started"))
    /**
     * 触发一次同步重试周期，重新执行失败/待恢复的任务。
     */
    suspend fun syncRetryFailed(): AppResult<Unit> = syncPlan?.retryFailed() ?: AppResult.Err(AppError.Internal("engine not started"))
    /**
     * 通知同步引擎网络已恢复，按需提交恢复周期。
     */
    fun syncNetworkRecovered() { syncPlan?.networkRecovered() }

    // ============ sync_status (4) ============
    /**
     * 基于 DB 最新数据计算并返回当前同步状态快照。
     */
    suspend fun syncState(): AppResult<SyncStatusSnapshot> = dbSafeSusp {
        statusAggregator.snapshot(db, statusAggregator.snapshots.value.runtime)
    }
    /**
     * 返回最近一次聚合的同步状态快照（不重新计算）。
     */
    suspend fun syncSnapshot(): AppResult<SyncStatusSnapshot> = safe { statusAggregator.snapshots.value }
    /**
     * 查询指定目录前缀下的所有同步项。
     */
    suspend fun syncItemsByFolder(folderLocalPath: String): AppResult<List<SyncItem>> = dbSafeSusp {
        val normalized = folderLocalPath.trim('/').takeIf(String::isNotBlank)
        db.syncItems.selectByFolderPrefix(normalized?.let { "$it/" }.orEmpty())
    }
    /**
     * 查询单个 fileId 对应的本地同步状态描述。
     */
    suspend fun syncCheckFileLocalStatus(fileId: String): AppResult<String> = dbSafeSusp {
        JvmSyncStatusResolver(configStore, db.syncItems).resolveOne(fileId)
    }
    /**
     * 批量查询多个 fileId 的本地同步状态。
     */
    suspend fun syncBatchFileStatus(fileIds: List<String>): AppResult<Map<String, String>> = dbSafeSusp {
        JvmSyncStatusResolver(configStore, db.syncItems).resolveBatch(fileIds)
    }

    // ============ transfer (6) ============
    /**
     * 列出全部传输任务记录。
     */
    suspend fun transferListAll(): AppResult<List<TransferTask>> = dbSafeSusp {
        db.transfers.selectAll()
    }
    /**
     * 判断是否存在仍在活动/待恢复的传输任务（用于阻塞更新安装等）。
     */
    fun transferHasActive(): Boolean = try {
        runBlocking {
            listOf(
                TransferState.Pending, TransferState.Running, TransferState.WaitingForNetwork,
                TransferState.BackingOff, TransferState.VerifyingRemote, TransferState.RestartRequired,
            ).any { db.transfers.selectByState(it).isNotEmpty() }
        }
    } catch (_: Throwable) {
        true
    }
    /**
     * 清除已完成（成功）的传输历史。
     */
    suspend fun transferClearCompleted(): AppResult<Unit> = clearTransferHistory(true, false)
    /**
     * 清除已失败的传输历史。
     */
    suspend fun transferClearFailed(): AppResult<Unit> = clearTransferHistory(false, true)
    /**
     * 清除所有已结束的传输历史（含成功和失败）。
     */
    suspend fun transferClearFinished(): AppResult<Unit> = clearTransferHistory(true, true)
    /**
     * 请求同步引擎重试指定 taskId 的传输任务。
     */
    suspend fun transferRetry(taskId: Long): AppResult<Unit> = dbSafeSusp {
        when (val result = syncPlan?.retryTransfer(taskId)) {
            is AppResult.Err -> throw result.error
            else -> Unit
        }
    }

    // ============ folder_sync (1) ============
    /**
     * 入队对指定云端目录的递归双向同步（后台 BFS 下载/上传）。
     */
    suspend fun syncFolderRecursive(folderId: String, relPath: String): AppResult<Long> = drive {
        val accepted = syncPlan?.enqueueFolderSync(folderId, relPath)
            ?: throw AppError.Internal("同步引擎未启动")
        if (!accepted) throw AppError.Internal("已有同步周期或目录同步正在运行，本次请求未开始")
        0L
    }

    // ============ free_up (5) ============
    /**
     * 校验某个文件是否可以安全释放本地空间（占位符化）。
     */
    suspend fun syncCheckSafeFreeUp(relPath: String, fileId: String): AppResult<String> = drive {
        freeUpService().checkSafe(relPath, fileId)
    }
    /**
     * 列出指定目录下所有可释放空间（已同步、有大小）的本地文件项。
     */
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
    /**
     * 释放单个文件占用的本地空间（校验路径一致后占位符化）。
     */
    suspend fun syncFreeUpSpace(fileId: String, relPath: String, localPath: String, name: String, size: Long): AppResult<Unit> = drive {
        val expected = configuredMountRoot().resolve(relPath).normalize()
        if (Paths.get(localPath).toAbsolutePath().normalize() != expected) {
            throw AppError.LocalIo("释放空间路径与 relPath 不一致")
        }
        freeUpService().freeOne(relPath, fileId, size)
        Unit
    }
    /**
     * 批量释放多个文件的本地空间，返回成功/失败统计与释放字节数。
     */
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
    /**
     * 按需下载云端文件到挂载目录，并标记本地文件为已下载（取消占位符）。
     */
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
    /**
     * 在 macOS Finder 中打开指定路径。
     */
    suspend fun platformOpenInFinder(path: String): AppResult<Boolean> = safe {
        ProcessBuilder("open", path).start()
        true
    }
    /**
     * 查询"开机启动"LaunchAgent 当前是否启用。
     */
    fun platformLaunchAtLoginIsEnabled(): Boolean = runCatching { launchAgentManager().isEnabled() }.getOrDefault(false)
    /**
     * 启用或禁用"开机启动"LaunchAgent，并返回设置是否生效。
     */
    fun platformLaunchAtLoginSetEnabled(enabled: Boolean): Boolean = runCatching {
        launchAgentManager().setEnabled(enabled)
        launchAgentManager().isEnabled() == enabled
    }.getOrDefault(false)
    /**
     * 清理全部本地缓存：停止同步、清空 token、DB 与同步状态/检查点文件。
     */
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
    /**
     * 读取最近 1000 条日志记录。
     */
    fun platformLogsList(): AppResult<List<io.github.yuanbaobaoo.petallink.core.logging.LogRecord>> = safe {
        val logger = io.github.yuanbaobaoo.petallink.core.logging.Logger()
        logger.snapshot(1000)
    }
    /**
     * 将日志导出到指定路径的文件。
     */
    fun platformLogsExport(path: String): AppResult<Unit> = safe {
        io.github.yuanbaobaoo.petallink.core.logging.LoggerRuntime.exportTo(Paths.get(path))
    }
    /**
     * 清空所有日志记录。
     */
    fun platformLogsClear(): AppResult<Unit> = safe {
        io.github.yuanbaobaoo.petallink.core.logging.LoggerRuntime.clear()
    }
    /**
     * 返回当前应用版本号。
     */
    fun platformAppGetVersion(): String = BuildInfo.VERSION
    /**
     * 检查是否有可用更新，返回更新清单（无则 null）。
     */
    suspend fun updaterCheck(): AppResult<UpdateManifest?> = drive { updateService.check() }
    /**
     * 下载并暂存更新包，随后启动安装器；活动传输存在时由 service 内部阻塞。
     */
    suspend fun updaterDownloadAndInstall(
        manifest: UpdateManifest,
        onProgress: (Long, Long?) -> Unit = { _, _ -> },
    ): AppResult<Boolean> = drive {
        val staged = updateService.downloadAndStage(manifest, ::transferHasActive, onProgress)
        updateService.launchInstaller(staged)
    }

    /**
     * 关闭服务：优雅停止同步引擎、关闭 HTTP 客户端与数据库。
     */
    fun close() {
        runBlocking { syncPlan?.closeGracefully() }
        httpClient.close()
        db.close()
    }

    /**
     * 读取配置并解析出挂载根目录；未配置或为空则抛错。
     */
    private fun configuredMountRoot(): Path {
        val config = configStore.load() ?: throw AppError.LocalIo("尚未配置挂载目录")
        if (!config.mountConfigured || config.mountDir.isBlank()) throw AppError.LocalIo("尚未配置挂载目录")
        return JvmMountPaths.resolve(config.mountDir)
    }

    /**
     * 重置账号运行时：停止同步、清空 DB、清除挂载配置并删除同步缓存文件。
     */
    private suspend fun resetAccountRuntimeAndMount() {
        syncPlan?.stop()
        db.clearAll()
        val current = configStore.load() ?: UserConfig()
        configStore.save(current.copy(mountDir = "", mountConfigured = false))
        clearSyncCacheFiles()
    }

    /**
     * 删除数据目录下的同步状态、云树与 changes 游标缓存文件。
     */
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

    /**
     * 构造释放空间服务，绑定当前挂载根目录、占位符管理与远端校验器。
     */
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

    /**
     * 构造开机启动 LaunchAgent 管理器，使用当前进程命令与 bundle id。
     */
    private fun launchAgentManager(): LaunchAgentManager {
        val command = ProcessHandle.current().info().command().orElseGet {
            Paths.get(System.getProperty("java.home"), "bin", "java").toString()
        }
        // label 随运行时 bundle id：dev 包注册 -dev plist，release 包注册 prod plist，互不覆盖。
        return LaunchAgentManager(AppPaths.currentBundleId(), Paths.get(command))
    }

    /**
     * 按完成/失败标志清除传输历史，并刷新同步状态快照。
     */
    private suspend fun clearTransferHistory(completed: Boolean, failed: Boolean): AppResult<Unit> = dbSafeSusp {
        db.transfers.clearHistory(completed, failed)
        statusAggregator.snapshot(db, statusAggregator.snapshots.value.runtime)
        Unit
    }

    /**
     * 在同步引擎的互斥锁内执行破坏性变更；引擎未启动时直接执行。
     */
    private suspend fun <T> exclusiveSyncMutation(block: suspend () -> T): T =
        syncPlan?.exclusiveMutation(block) ?: block()

    /**
     * 执行上传命令：创建传输任务、运行至完成，并返回远端文件元数据。
     */
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
        val disposition = commandTaskRunner(store).runExpected(db.transfers.findById(id)!!.toTaskContext())
        if (disposition != TaskDisposition.COMPLETED) {
            val task = db.transfers.findById(id)
            throw AppError.Internal(task?.errorMessage ?: "上传未完成: $disposition")
        }
        val remoteId = db.transfers.findById(id)?.remoteResultFileId
            ?: throw AppError.Data("上传完成但缺少 remote_result_file_id")
        return filesApi.getFile(remoteId)
    }

    /**
     * 执行下载命令：创建下载（或下载更新）传输任务并运行至完成。
     */
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
        val disposition = commandTaskRunner(store).runExpected(db.transfers.findById(id)!!.toTaskContext())
        if (disposition != TaskDisposition.COMPLETED) {
            val task = db.transfers.findById(id)
            throw AppError.Internal(task?.errorMessage ?: "下载未完成: $disposition")
        }
    }

    /**
     * 校验命令传入的路径：必须位于挂载根目录内、非符号链接，返回 (根, 相对路径, 绝对路径)。
     */
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

    /**
     * 构造命令传输任务执行器，绑定上传/下载 API 与本地文件读写实现。
     */
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
            ensureUploadCapacity = aboutApi::ensureUploadCapacity,
        )
        val concurrency = (configStore.load() ?: UserConfig()).concurrency
        return TaskRunner(
            db.transfers,
            operations,
            { netGuard?.state?.value != NetState.OFFLINE },
            System::currentTimeMillis,
            maxConcurrentTransfers = concurrency,
            onTaskChanged = { taskId ->
                val revision = db.transfers.findById(taskId)?.stateRevision ?: 0L
                eventHub.publishTransferUpdate(TransferUpdateEvent(taskId, revision))
            },
        )
    }

    // ============ helpers ============
    /**
     * 包装同步块：将任意异常转为 [AppResult.Err]（Internal）。
     */
    private fun <T> safe(block: () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Internal(e.message ?: "unknown")) }
    /**
     * 包装云端操作：AppError 透传，其他异常归为 Remote 错误。
     */
    private suspend fun <T> drive(block: suspend () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: AppError) { AppResult.Err(e) } catch (e: Throwable) { AppResult.Err(AppError.Remote(0, e.message ?: "drive error")) }
    /**
     * 包装本地 DB 操作（非挂起）：异常归为 Data 错误。
     */
    private fun <T> dbSafe(block: () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Data(e.message ?: "db error")) }
    /**
     * 包装挂起的 DB 操作：异常归为 Data 错误。
     */
    private suspend fun <T> dbSafeSusp(block: suspend () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Data(e.message ?: "db error")) }

    companion object {
        /**
         * 工厂方法：创建完整的 CommandService 并自动布线 service 链。
         */
        fun create(paths: AppPaths = AppPaths.fromEnvironment(), netGuard: NetGuard? = null): CommandService {
            io.github.yuanbaobaoo.petallink.core.logging.LoggerRuntime.configure(paths.logsDir)
            val httpClient = HttpClient(CIO) {
                engine { requestTimeout = 60_000; endpoint.connectTimeout = 15_000 }
            }
            val envLoader = EnvLoader.apply { loadEnvFile() }
            val configStore = JsonConfigStore(paths.configFile)
            val db = PetalLinkDb(paths.databaseFile.toString())
            val statusAgg = StatusAggregator()
            val eventHub = SyncEventHub(statusAgg)

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
            val driveClient = DriveClient(
                httpClient,
                provider,
                { runBlocking { tokenRefresher.refresh() } },
                { netGuard?.reportRequestNetworkFailure() },
            )
            val filesApi = FilesApi(driveClient)
            val changesApi = ChangesApi(driveClient)
            val downloadApi = DownloadApi(driveClient)
            val uploadApi = UploadApi(driveClient)
            val thumbnailApi = ThumbnailApi(driveClient)
            val aboutApi = AboutApi(driveClient)
            val syncPlan = JvmSyncRuntime(
                paths,
                configStore,
                db,
                filesApi,
                changesApi,
                uploadApi,
                downloadApi,
                statusAgg,
                aboutApi::ensureUploadCapacity,
                { netGuard?.state?.value != NetState.OFFLINE },
                { taskId ->
                    val revision = db.transfers.findById(taskId)?.stateRevision ?: 0L
                    eventHub.publishTransferUpdate(TransferUpdateEvent(taskId, revision))
                },
            )
            val updateService = JvmUpdateService(
                httpClient, paths, BuildInfo.VERSION, BuildInfo.UPDATE_ENDPOINT, BuildInfo.UPDATE_TEAM_ID,
            )

            return CommandService(
                configStore, db, httpClient, tokenStore, authService, userInfoApi,
                filesApi, changesApi, downloadApi, uploadApi, thumbnailApi, aboutApi,
                driveClient,
                tokenRefresher,
                statusAgg,
                envLoader,
                syncPlan,
                paths,
                updateService,
                netGuard,
                eventHub,
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

    /**
     * 用 IOPlatformUUID 派生加密密钥（对标原项目 machine-bound）
     */
    private fun deriveKey(): ByteArray {
        val uuid = readMachineUUID()
            ?: throw AppError.Auth("无法读取 IOPlatformUUID，拒绝使用不安全的降级密钥")
        val digest = java.security.MessageDigest.getInstance("SHA-256")
        return digest.digest(uuid.toByteArray())
    }

    /**
     * 通过 ioreg 读取本机 IOPlatformUUID，失败返回 null。
     */
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

    /**
     * 读取并解密本地 token 文件，返回 TokenPair；文件不存在或解密失败返回 null。
     */
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
            val plaintext = io.github.yuanbaobaoo.petallink.platform.ChaCha20Poly1305.decrypt(key, nonce, ciphertext)
            val tokenPair = TokenSerializer.deserialize(plaintext)
            tokenPair
        } catch (e: Throwable) { null }
    }

    /**
     * 加密并原子写入 token 到本地文件，权限为 rw-------。
     */
    override suspend fun save(token: TokenPair) {
        val path = file.toPath()
        Files.createDirectories(path.parent)
        val plaintext = TokenSerializer.serialize(token)
        val key = deriveKey()
        // 随机 nonce（每次保存重新生成）
        val nonce = randomBytes(nonceLen)
        val ciphertext = io.github.yuanbaobaoo.petallink.platform.ChaCha20Poly1305.encrypt(key, nonce, plaintext)
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

    /**
     * 删除本地 token 文件。
     */
    override suspend fun clear() { file.delete() }

    /**
     * 以 runBlocking 阻塞方式加载 token，供非挂起上下文使用。
     */
    fun loadSuspended(): TokenPair? = runBlocking { load() }
    /**
     * 使用 SecureRandom 生成指定长度的随机字节。
     */
    private fun randomBytes(n: Int) = java.security.SecureRandom().run { ByteArray(n).also { nextBytes(it) } }

}

// ============ AppResult 类型 ============
sealed class AppResult<out T> {
    /**
     * 命令执行成功，携带返回值。
     */
    data class Ok<T>(val value: T) : AppResult<T>()
    /**
     * 命令执行失败，携带错误信息。
     */
    data class Err(val error: AppError) : AppResult<Nothing>()
}

/**
 * 鉴权状态：登录态、OAuth secret 是否就绪、回调端口。
 */
data class AuthState(val loggedIn: Boolean, val secretConfigured: Boolean, val callbackPort: Int)
/**
 * 云端文件列表查询结果，含本页文件和分页游标。
 */
data class FileListResult(val files: List<io.github.yuanbaobaoo.petallink.drive.DriveFile>, val nextCursor: String?)
/**
 * 可释放空间的本地文件项，含 fileId、相对/绝对路径、名称与大小。
 */
data class FreeableItem(val fileId: String, val relPath: String, val localPath: String, val name: String, val size: Long)
/**
 * 批量释放空间结果，含成功/跳过数量、释放字节数与错误列表。
 */
data class FreeUpBatchResult(
    val freedCount: Int,
    val skippedCount: Int,
    val freedBytes: Long,
    val errors: List<String>,
)
/**
 * 文件夹同步进度，done 为已完成数，total 为总数。
 */
data class FolderSyncProgress(val done: Int, val total: Int)

/**
 * 同步命令执行计划；封装同步引擎的手动刷新、重试、配置变更与文件夹同步生命周期。
 */
interface SyncCommandPlan : AutoCloseable {
    /**
     * 触发一次本地+全量云端的同步周期。
     */
    suspend fun manualRefresh(): AppResult<Unit>
    /**
     * 触发一次增量云端+本地+重试的同步周期。
     */
    suspend fun retryFailed(): AppResult<Unit>
    /**
     * 请求重试指定 taskId 的传输；默认等同于全量重试。
     */
    suspend fun retryTransfer(taskId: Long): AppResult<Unit> = retryFailed()
    /**
     * 预告配置即将变更：暂停同步源、标记重配置中。
     */
    fun prepareConfigurationChange() = Unit
    /**
     * 配置已变更：按需切换挂载目录并重新启动同步源。
     */
    fun configurationChanged(previous: UserConfig, current: UserConfig) = Unit
    /**
     * 配置保存失败后的回滚通知。
     */
    fun configurationChangeFailed() = Unit
    /**
     * 启动同步引擎。
     */
    fun start() = Unit
    /**
     * 停止同步引擎。
     */
    fun stop() = Unit
    /**
     * 通知引擎网络已恢复，可提交恢复周期。
     */
    fun networkRecovered() = Unit
    /**
     * 在同步引擎互斥保护下执行破坏性变更块。
     */
    suspend fun <T> exclusiveMutation(block: suspend () -> T): T = block()
    /**
     * 通知引擎远端已发生写操作，需重新对账。
     */
    fun remoteMutationCommitted() = Unit
    /**
     * 入队对指定云端目录的递归同步；返回是否被接受。
     */
    fun enqueueFolderSync(folderId: String, relativePath: String): Boolean = false
    /**
     * 目录同步进度流（done/total）。
     */
    fun folderSyncProgress(): kotlinx.coroutines.flow.StateFlow<FolderSyncProgress?>? = null
    /**
     * 上传失败事件流。
     */
    fun uploadFailures(): kotlinx.coroutines.flow.SharedFlow<UploadFailedEvent>? = null
    /**
     * 在超时时间内优雅关闭同步引擎，返回是否在超时前完成。
     */
    suspend fun closeGracefully(timeoutMs: Long = 3_200L): Boolean {
        close()
        return true
    }
    /**
     * 释放同步引擎持有的资源。
     */
    override fun close() = Unit
}
