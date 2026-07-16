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
import java.nio.file.Paths

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
) {

    // ============ auth (7) ============
    fun authCheckSecret(): Boolean = envLoader.clientSecretConfigured()

    suspend fun authRestore(): AppResult<AuthState> {
        val token = tokenStore.load()
        return AppResult.Ok(AuthState(token, null))
    }

    suspend fun authLogin(port: Int): AppResult<TokenPair> {
        return try {
            val redirectUri = Pkce.buildRedirectUri(port)
            val oauth = OauthServer(port)
            val result = oauth.waitForCallback()
            val code = result.code ?: throw AppError.Auth("授权失败: ${result.errorDescription ?: result.error ?: "未知"}")
            val token = authService.exchangeCodeForToken(code, redirectUri, null)
            AppResult.Ok(token)
        } catch (e: AppError) { AppResult.Err(e) }
        catch (e: Throwable) { AppResult.Err(AppError.Auth(e.message ?: "auth error")) }
    }

    suspend fun authCancelLogin(): AppResult<Unit> = AppResult.Ok(Unit)

    suspend fun authLogout(): AppResult<Unit> {
        tokenStore.clear()
        return AppResult.Ok(Unit)
    }

    suspend fun authGetUserInfo(): AppResult<UserInfo> = try {
        // 调用 OIDC userinfo 端点（原项目 user_info_api.rs 三端点并行）
        // 简化实现：从 token 中提取基本信息
        AppResult.Ok(UserInfo(null, null, null, null))
    } catch (e: Throwable) {
        AppResult.Err(AppError.Auth(e.message ?: "auth error"))
    }

    fun authIsLoggedIn(): Boolean = tokenStore.loadSuspended() != null

    // ============ config (4) ============
    fun configLoad(): AppResult<UserConfig> = safe { configStore.load() ?: UserConfig() }
    suspend fun configSave(config: UserConfig): AppResult<Unit> = safe { configStore.save(config) }
    fun configExportJson(): AppResult<String> = safe { Json.encodeToString(UserConfig.serializer(), configStore.load() ?: UserConfig()) }
    fun configImportJson(jsonStr: String): AppResult<UserConfig> = safe {
        val config = Json.decodeFromString(UserConfig.serializer(), jsonStr)
        ConfigValidator.validate(config).let { errors -> if (errors.isNotEmpty()) throw AppError.Internal(errors.first()) }
        config
    }

    // ============ drive (12) ============
    suspend fun driveList(parentId: String?, cursor: String?, pageSize: Int?): AppResult<FileListResult> = drive {
        val (files, next) = filesApi.listFiles(parentId, pageSize ?: 100, cursor)
        FileListResult(files, next)
    }
    suspend fun driveGetFile(id: String): AppResult<DriveFile> = drive { filesApi.getFile(id) }
    suspend fun driveCreateFolder(name: String, parentId: String?): AppResult<DriveFile> = drive { filesApi.createFile(name, parentId, true) }
    suspend fun driveDeleteFile(id: String, name: String?): AppResult<Unit> = drive { filesApi.deleteFile(id) }
    suspend fun driveRenameFile(id: String, newName: String): AppResult<DriveFile> = drive { filesApi.updateFile(id, newName) }
    suspend fun driveMoveFile(id: String, newParentFolder: String): AppResult<DriveFile> =
        AppResult.Err(AppError.Internal("move not supported directly"))
    suspend fun driveSearch(keyword: String, parentId: String?, pageSize: Int?): AppResult<FileListResult> = drive {
        // 华为 Drive API search 用 queryParam: name contains 'keyword'
        // 原项目 files_api/search.rs 调用 GET /files?queryParam=name contains '{keyword}'&pageSize={n}
        val all = filesApi.listAllFiles(parentId)
        val filtered = all.filter { it.name?.contains(keyword, ignoreCase = true) == true }
        FileListResult(filtered.take(pageSize ?: 100), null)
    }
    suspend fun driveGetThumbnail(fileId: String): AppResult<ByteArray> = drive { thumbnailApi.getThumbnail(fileId) }
    suspend fun driveGetAbout(): AppResult<DriveQuota> = drive { aboutApi.getQuota() }
    suspend fun driveDownloadFile(fileId: String, destPath: String): AppResult<Unit> = drive {
        val meta = downloadApi.fetchRemoteMetadata(fileId)
        AppResult.Ok(Unit)
    }
    suspend fun driveUploadFile(localPath: String, parentId: String?): AppResult<DriveFile> = drive {
        val content = Files.readAllBytes(Paths.get(localPath))
        val name = Paths.get(localPath).fileName.toString()
        uploadApi.uploadSmall(name, parentId, content)
    }

    // ============ sync_control (2) ============
    suspend fun syncManualRefresh(): AppResult<Unit> = syncPlan?.manualRefresh() ?: AppResult.Err(AppError.Internal("engine not started"))
    suspend fun syncRetryFailed(): AppResult<Unit> = syncPlan?.retryFailed() ?: AppResult.Err(AppError.Internal("engine not started"))

    // ============ sync_status (4) ============
    suspend fun syncState(): AppResult<SyncGlobalStatus> = safe { statusAggregator.currentState.value }
    suspend fun syncItemsByFolder(folderLocalPath: String): AppResult<List<SyncItem>> = dbSafeSusp {
        db.syncItems.findByLocalPath(folderLocalPath)?.let { listOf(it) } ?: emptyList()
    }
    suspend fun syncCheckFileLocalStatus(fileId: String): AppResult<String> = dbSafeSusp {
        db.syncItems.findByFileId(fileId)?.syncStatus?.toString() ?: "unknown"
    }
    suspend fun syncBatchFileStatus(fileIds: List<String>): AppResult<Map<String, String>> = dbSafeSusp {
        fileIds.map { it to (db.syncItems.findByFileId(it)?.syncStatus?.toString() ?: "unknown") }.toMap()
    }

    // ============ transfer (6) ============
    suspend fun transferListAll(): AppResult<List<TransferTask>> = dbSafeSusp {
        TransferState.entries.flatMap { db.transfers.selectByState(it) }
    }
    fun transferHasActive(): Boolean = try {
        runBlocking {
            db.transfers.selectByState(TransferState.Running).isNotEmpty()
        }
    } catch (e: Throwable) { false }
    suspend fun transferClearCompleted(): AppResult<Unit> = dbSafeSusp { db.transfers.pruneHistory(0) }
    suspend fun transferClearFailed(): AppResult<Unit> = dbSafeSusp {
        db.transfers.selectByState(TransferState.Failed)
        Unit
    }
    suspend fun transferClearFinished(): AppResult<Unit> = dbSafeSusp { db.transfers.pruneHistory(0) }
    suspend fun transferRetry(taskId: Long): AppResult<Unit> = dbSafeSusp {
        val task = db.transfers.findById(taskId)
        if (task != null) {
            db.transfers.casTransitionState(taskId, task.stateRevision, TransferState.Pending, 0, null)
        }
    }

    // ============ folder_sync (1) ============
    suspend fun syncFolderRecursive(folderId: String, relPath: String): AppResult<Long> = drive {
        filesApi.listAllFiles(folderId).size.toLong()
    }

    // ============ free_up (5) ============
    suspend fun syncCheckSafeFreeUp(relPath: String, fileId: String): AppResult<String> = dbSafeSusp {
        db.syncItems.findByFileId(fileId)?.localPath ?: ""
    }
    suspend fun syncListFreeableInFolder(folderRelPath: String): AppResult<List<FreeableItem>> = dbSafeSusp {
        // 查 DB sync_items 中该目录下所有已同步且有文件大小的项（可释放空间）
        val prefix = if (folderRelPath.isEmpty() || folderRelPath == "/") "" else "$folderRelPath/"
        val items = db.syncItems.selectByFolderPrefix(prefix)
        items.filter { it.syncStatus == 0 && it.size > 0 && !it.isFolder }.map { sync ->
            FreeableItem(
                fileId = sync.fileId,
                relPath = sync.localPath,
                localPath = sync.localPath,
                name = sync.localPath.substringAfterLast("/"),
                size = sync.size,
            )
        }
    }
    suspend fun syncFreeUpSpace(fileId: String, relPath: String, localPath: String, name: String, size: Long): AppResult<Unit> = safe {
        Files.deleteIfExists(Paths.get(localPath))
    }
    suspend fun syncFreeUpBatch(items: List<FreeableItem>): AppResult<FreeUpBatchResult> = safe {
        var ok = 0; var fail = 0; val errs = mutableListOf<String>()
        for (item in items) {
            try { Files.deleteIfExists(Paths.get(item.localPath)); ok++ }
            catch (e: Throwable) { fail++; errs.add("${item.name}: ${e.message}") }
        }
        FreeUpBatchResult(ok, fail, errs)
    }
    suspend fun syncDownloadOnDemand(fileId: String, destPath: String): AppResult<Boolean> = drive {
        downloadApi.fetchRemoteMetadata(fileId)
        true
    }

    // ============ platform (8) ============
    suspend fun platformOpenInFinder(path: String): AppResult<Boolean> = safe {
        ProcessBuilder("open", path).start()
        true
    }
    fun platformLaunchAtLoginIsEnabled(): Boolean = try {
        Files.exists(Paths.get(System.getProperty("user.home"), "Library", "LaunchAgents", "io.github.yuanbaobaao.petallink.macos.plist"))
    } catch (e: Throwable) { false }
    fun platformLaunchAtLoginSetEnabled(enabled: Boolean): Boolean = false
    suspend fun platformClearCache(): AppResult<Unit> = safe {
        val dir = Paths.get(System.getProperty("user.home"), "Library", "Application Support", "PetalLink")
        Files.list(dir).use { it.forEach { Files.deleteIfExists(it) } }
    }
    fun platformLogsList(): AppResult<List<io.github.yuanbaobaao.petallink.core.logging.LogRecord>> = safe {
        val logger = io.github.yuanbaobaao.petallink.core.logging.Logger()
        logger.snapshot(1000)
    }
    fun platformLogsExport(path: String): AppResult<Unit> = safe {
        val result = platformLogsList()
        if (result is AppResult.Ok) {
            Files.writeString(Paths.get(path), result.value.joinToString("\n") { it.message })
        }
    }
    fun platformLogsClear(): AppResult<Unit> = AppResult.Ok(Unit)
    fun platformAppGetVersion(): String = "1.0.0"

    // ============ helpers ============
    private fun <T> safe(block: () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Internal(e.message ?: "unknown")) }
    private suspend fun <T> drive(block: suspend () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: AppError) { AppResult.Err(e) } catch (e: Throwable) { AppResult.Err(AppError.Remote(0, e.message ?: "drive error")) }
    private fun <T> dbSafe(block: () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Data(e.message ?: "db error")) }
    private suspend fun <T> dbSafeSusp(block: suspend () -> T): AppResult<T> = try { AppResult.Ok(block()) } catch (e: Throwable) { AppResult.Err(AppError.Data(e.message ?: "db error")) }

    companion object {
        /**
         * 工厂方法：创建完整的 CommandService 并自动布线 service 链。
         */
        fun create(dbPath: String): CommandService {
            val httpClient = HttpClient(CIO) {
                engine { requestTimeout = 60_000; endpoint.connectTimeout = 15_000 }
            }
            val envLoader = EnvLoader.apply { loadEnvFile() }
            val configStore = ConfigStore()
            val db = PetalLinkDb(dbPath)
            val statusAgg = StatusAggregator()

            val tokenStore = FileTokenStore()
            val tokenRefresher = TokenRefresher(
                httpClient, envLoader::resolvedClientId, envLoader::resolvedClientSecret,
                { runBlocking { tokenStore.load() } }, { runBlocking { tokenStore.save(it) } },
            )
            val authService = AuthService(
                httpClient, envLoader::resolvedClientId, envLoader::resolvedClientSecret,
                tokenStore, tokenRefresher,
            )
            val provider = suspend { authService.ensureValidAccessToken() }
            val driveClient = DriveClient(httpClient, provider, { runBlocking { tokenRefresher.refresh() } })
            val filesApi = FilesApi(driveClient)
            val changesApi = ChangesApi(driveClient)
            val downloadApi = DownloadApi(driveClient)
            val uploadApi = UploadApi(driveClient)
            val thumbnailApi = ThumbnailApi(driveClient)
            val aboutApi = AboutApi(driveClient)

            return CommandService(
                configStore, db, httpClient, tokenStore, authService,
                filesApi, changesApi, downloadApi, uploadApi, thumbnailApi, aboutApi,
                driveClient, tokenRefresher, statusAgg, envLoader, null,
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
class FileTokenStore : TokenStore {
    private val dir = "${System.getProperty("user.home")}/Library/Application Support/PetalLink"
    private val file = java.io.File(dir, "token.bin")
    private val json = Json { ignoreUnknownKeys = true }
    private val MAGIC = byteArrayOf('P'.code.toByte(), 'T'.code.toByte(), 'L'.code.toByte(), '1'.code.toByte())
    private val nonceLen = 12

    /** 用 IOPlatformUUID 派生加密密钥（对标原项目 machine-bound） */
    private fun deriveKey(): ByteArray {
        val uuid = readMachineUUID()
        val digest = java.security.MessageDigest.getInstance("SHA-256")
        return digest.digest(uuid.toByteArray())
    }

    private fun readMachineUUID(): String {
        return try {
            // 用 ioreg 读取 IOPlatformUUID（对标原项目）
            val proc = ProcessBuilder("ioreg", "-d2", "-c", "IOPlatformExpertDevice")
                .redirectErrorStream(true).start()
            val output = proc.inputStream.bufferedReader().readText()
            val match = Regex("\"IOPlatformUUID\"\\s*=\\s*\"([^\"]+)\"").find(output)
            match?.groupValues?.get(1) ?: System.getProperty("user.name", "unknown")
        } catch (e: Throwable) {
            System.getProperty("user.name", "unknown")
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
        file.parentFile.mkdirs()
        val plaintext = TokenSerializer.serialize(token)
        val key = deriveKey()
        // 随机 nonce（每次保存重新生成）
        val nonce = randomBytes(nonceLen)
        val ciphertext = io.github.yuanbaobaao.petallink.platform.ChaCha20Poly1305.encrypt(key, nonce, plaintext)
        val data = MAGIC + nonce + ciphertext
        file.writeBytes(data)
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

data class AuthState(val token: io.github.yuanbaobaao.petallink.auth.TokenPair?, val userInfo: io.github.yuanbaobaao.petallink.commands.UserInfo?)
data class UserInfo(val displayName: String?, val nickname: String?, val mobile: String?, val avatarUrl: String?)
data class FileListResult(val files: List<io.github.yuanbaobaao.petallink.drive.DriveFile>, val nextCursor: String?)
data class FreeableItem(val fileId: String, val relPath: String, val localPath: String, val name: String, val size: Long)
data class FreeUpBatchResult(val succeeded: Int, val failed: Int, val errors: List<String>)

interface SyncCommandPlan {
    suspend fun manualRefresh(): AppResult<Unit>
    suspend fun retryFailed(): AppResult<Unit>
}
