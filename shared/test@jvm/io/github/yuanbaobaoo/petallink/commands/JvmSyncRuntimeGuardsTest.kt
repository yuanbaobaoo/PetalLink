package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.PlatformInode
import io.github.yuanbaobaoo.petallink.auth.TokenPair
import io.github.yuanbaobaoo.petallink.config.ConfigStore
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.drive.ChangesApi
import io.github.yuanbaobaoo.petallink.drive.DownloadApi
import io.github.yuanbaobaoo.petallink.drive.DriveClient
import io.github.yuanbaobaoo.petallink.drive.FilesApi
import io.github.yuanbaobaoo.petallink.drive.UploadApi
import io.github.yuanbaobaoo.petallink.mount.JvmPlaceholderManager
import io.github.yuanbaobaoo.petallink.mount.MacXattrAccess
import io.github.yuanbaobaoo.petallink.sync.SyncStatus
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.sync.engine.StatusAggregator
import io.github.yuanbaobaoo.petallink.sync.engine.UploadStability
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpMethod
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.time.Instant
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import kotlin.test.assertFalse

/**
 * 同步结算层安全护栏的回归测试（对照原项目 settlement/admission/results/local_delete/
 * reconciliation/actions/transfer_operations 的语义移植）。
 */
class JvmSyncRuntimeGuardsTest {
    private val editedT1 = "2026-01-01T00:00:00Z"
    private val editedT2 = "2026-01-02T00:00:00Z"
    private val editedT1Ms: Long = Instant.parse(editedT1).toEpochMilli()

    private fun remoteFile(id: String, name: String, size: Long, editedTime: String?, parent: String = "root"): String {
        val edited = editedTime?.let { ""","editedTime":"$it"""" } ?: ""
        return """{"category":"drive#file","id":"$id","fileName":"$name","mimeType":"text/plain","parentFolder":["$parent"],"size":"$size"$edited}"""
    }

    private fun remoteList(vararg files: String): String =
        """{"category":"drive#fileList","files":[${files.joinToString(",")}]}"""

    private fun workspace(): Triple<Path, Path, Path> {
        val root = Files.createTempDirectory("petallink-guards-")
        return Triple(root, Files.createDirectory(root.resolve("mount")), Files.createDirectory(root.resolve("data")))
    }

    private fun memoryConfig(config: UserConfig): ConfigStore = object : ConfigStore {
        private var value = config
        override fun load(): UserConfig = value
        override fun save(config: UserConfig) { value = config }
    }

    private fun newRuntime(paths: AppPaths, config: ConfigStore, db: PetalLinkDb, client: DriveClient) =
        JvmSyncRuntime(
            paths, config, db,
            FilesApi(client, "https://example.test/drive/v1"),
            ChangesApi(client, "https://example.test/drive/v1"),
            UploadApi(client, "https://example.test/upload/drive/v1"),
            DownloadApi(client, "https://example.test/drive/v1"),
            StatusAggregator(),
            stabilityFactory = { { UploadStability.STABLE } },
        )

    private suspend fun seedBaseline(
        db: PetalLinkDb,
        fileId: String,
        path: String,
        localMtime: Long?,
        localSize: Long?,
        cloudEditedTime: Long? = editedT1Ms,
        status: Int = SyncStatus.SYNCED,
    ) = db.syncItems.upsert(
        SyncItem(
            fileId = fileId,
            localPath = path,
            parentFolderId = "root",
            name = path.substringAfterLast('/'),
            isFolder = false,
            size = localSize ?: 0L,
            localSize = localSize,
            sha256 = null,
            localMtime = localMtime,
            cloudEditedTime = cloudEditedTime,
            lastSyncTime = 0L,
            status = status,
            errorMessage = null,
        ),
    )

    private fun stat(path: Path): Pair<Long, Long> =
        Files.getLastModifiedTime(path, LinkOption.NOFOLLOW_LINKS).toMillis() to Files.size(path)

    /**
     * §3.2（原 settlement.rs:240-251）：上传成功结算必须记录任务里持久化的源快照，
     * 上传窗口内的本地编辑不得被基线吞掉。
     */
    @Test
    fun 上传期间本地被编辑时基线记录源快照而非结算时刻stat() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("race.txt"), "v1")
        val (mtimeBefore, sizeBefore) = stat(mount.resolve("race.txt"))
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Post && url.contains("uploadType=multipart") -> {
                    // 模拟上传网络窗口内的并发编辑
                    Files.writeString(mount.resolve("race.txt"), "v2-edited")
                    remoteFile("up1", "race.txt", 2L, editedT1)
                }
                url.contains("/files?") -> remoteList()
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            val baseline = assertNotNull(db.syncItems.findByFileId("up1"))
            assertEquals(sizeBefore, baseline.localSize, "基线 localSize 必须来自上传前持久化的源快照")
            assertEquals(mtimeBefore, baseline.localMtime, "基线 localMtime 必须来自上传前持久化的源快照")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §3.4（原 admission.rs:166-174,292-414）：新一轮 planner intent 到达时 RestartRequired
     * 任务自动重规划，不再只等手动重试。
     */
    @Test
    fun RestartRequired任务在新意图到达时自动重规划并续传() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("edit.txt"), "new-content")
        var updateCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Get && url.contains("/files/f1") ->
                    remoteFile("f1", "edit.txt", 11L, editedT1)
                request.method == HttpMethod.Patch && url.contains("uploadType=multipart") -> {
                    updateCalls++
                    remoteFile("f1", "edit.txt", 11L, editedT2)
                }
                url.contains("/files?") -> remoteList(remoteFile("f1", "edit.txt", 5L, editedT1))
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "edit.txt", localMtime = 1L, localSize = 1L)
        val staleTaskId = db.transfers.insert(
            TransferTask(
                id = null,
                direction = TransferDirection.UPLOAD,
                fileId = "f1",
                localPath = mount.resolve("edit.txt").toString(),
                name = "edit.txt",
                totalSize = 1L,
                state = TransferState.RestartRequired,
                errorMessage = "文件尚不稳定，等待重新规划",
                createdAt = System.currentTimeMillis(),
                relativePath = "edit.txt",
                parentFileId = "root",
                operation = 1,
                sourceMtime = 1L,
                sourceSize = 1L,
                expectedCloudEditedTime = editedT1Ms,
            ),
        )
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertEquals(TransferState.Canceled, db.transfers.findById(staleTaskId)?.state, "旧意图任务应被重规划取消")
            val tasks = db.transfers.selectAll()
            assertEquals(2, tasks.size, "重规划应以全新 intent 新建任务")
            assertEquals(1, updateCalls, "重规划后的新任务应立即续传")
            val baseline = assertNotNull(db.syncItems.findByFileId("f1"))
            assertEquals(11L, baseline.localSize, "续传成功后基线应记录新源快照")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §3.4（原 admission.rs:153-158）：含持久化远端结果的 RestartRequired 禁止重放，
     * 只提升为待核验。
     */
    @Test
    fun 含远端结果的RestartRequired任务只提升为待核验() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("edit.txt"), "new-content")
        var updateCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Patch && url.contains("uploadType=multipart") -> {
                    updateCalls++
                    remoteFile("f1", "edit.txt", 11L, editedT2)
                }
                url.contains("/files?") -> remoteList(remoteFile("f1", "edit.txt", 5L, editedT1))
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "edit.txt", localMtime = 1L, localSize = 1L)
        val staleTaskId = db.transfers.insert(
            TransferTask(
                id = null,
                direction = TransferDirection.UPLOAD,
                fileId = "f1",
                localPath = mount.resolve("edit.txt").toString(),
                name = "edit.txt",
                totalSize = 1L,
                state = TransferState.RestartRequired,
                errorMessage = "远端写入结果不确定",
                createdAt = System.currentTimeMillis(),
                relativePath = "edit.txt",
                parentFileId = "root",
                operation = 1,
                sourceMtime = 1L,
                sourceSize = 1L,
                expectedCloudEditedTime = editedT1Ms,
                remoteResultFileId = "rid",
            ),
        )
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertEquals(
                TransferState.VerifyingRemote, db.transfers.findById(staleTaskId)?.state,
                "含远端结果的重规划任务应提升为待核验",
            )
            assertEquals(0, updateCalls, "歧义远端结果禁止重放上传")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §3.9（原 results.rs:106-130）：动作失败落 sync_items 为 FAILED 状态。
     */
    @Test
    fun 云端删除失败落FAILED状态() = runBlocking {
        val (_, mount, data) = workspace()
        val engine = MockEngine { request ->
            val url = request.url.toString()
            when {
                url.contains("getStartCursor") ->
                    respond("""{"category":"drive#startCursor","startCursor":"s0"}""", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                url.contains("/changes?") ->
                    respond("""{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}""", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                request.method == HttpMethod.Patch ->
                    respond("{}", HttpStatusCode.InternalServerError, headersOf(HttpHeaders.ContentType, "application/json"))
                request.method == HttpMethod.Get && url.contains("/files/f1") ->
                    respond(remoteFile("f1", "gone.txt", 5L, editedT1), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                url.contains("/files?") ->
                    respond(remoteList(remoteFile("f1", "gone.txt", 5L, editedT1)), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                else -> error("未预期请求: ${request.method.value} $url")
            }
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "gone.txt", localMtime = 1L, localSize = 5L)
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Err>(runtime.manualRefresh())
            val baseline = assertNotNull(db.syncItems.findByFileId("f1"))
            assertEquals(SyncStatus.FAILED, baseline.status, "删除失败应落 FAILED 状态")
            assertTrue(!baseline.errorMessage.isNullOrBlank(), "FAILED 状态必须携带错误消息")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §3.9（原 admission.rs:175-181）：失败任务作为路径屏障，下一轮不再无条件重放新建任务。
     */
    @Test
    fun 失败上传任务形成屏障下轮不再新建任务() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("broken.txt"), "changed")
        var updateCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            when {
                url.contains("getStartCursor") ->
                    respond("""{"category":"drive#startCursor","startCursor":"s0"}""", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                url.contains("/changes?") ->
                    respond("""{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}""", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                request.method == HttpMethod.Get && url.contains("/files/f1") ->
                    respond(remoteFile("f1", "broken.txt", 7L, editedT1), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                request.method == HttpMethod.Patch && url.contains("uploadType=multipart") -> {
                    updateCalls++
                    respond("{}", HttpStatusCode.BadRequest, headersOf(HttpHeaders.ContentType, "application/json"))
                }
                url.contains("/files?") ->
                    respond(remoteList(remoteFile("f1", "broken.txt", 5L, editedT1)), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                else -> error("未预期请求: ${request.method.value} $url")
            }
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "broken.txt", localMtime = 1L, localSize = 1L)
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Err>(runtime.manualRefresh())
            assertEquals(1, updateCalls)
            assertEquals(1, db.transfers.selectAll().size)
            assertEquals(SyncStatus.FAILED, db.syncItems.findByFileId("f1")?.status)

            // 屏障：下一轮周期复用既有失败任务并延期，不再新建任务热循环
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertEquals(1, updateCalls, "屏障期内禁止重放失败上传")
            assertEquals(1, db.transfers.selectAll().size, "屏障期内禁止新建传输任务")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §4.4（原 local_delete.rs:167-181）：云端文件仍存在（verify_deleted=false）时取消本地删除。
     */
    @Test
    fun 云端文件仍存在时取消本地删除() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("keep.txt"), "keep")
        val (mtime, size) = stat(mount.resolve("keep.txt"))
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Get && url.contains("/files/f1") ->
                    remoteFile("f1", "keep.txt", 4L, editedT1)
                url.contains("/files?") -> remoteList()
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "keep.txt", localMtime = mtime, localSize = size)
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertTrue(Files.exists(mount.resolve("keep.txt")), "云端未确认删除时本地内容必须保留")
            assertEquals("keep", Files.readString(mount.resolve("keep.txt")))
            assertNotNull(db.syncItems.findByFileId("f1"), "取消删除不得推进基线")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §4.4（原 local_delete.rs:183-209）：远端核验返回后复核本地快照，期间变化则取消删除。
     */
    @Test
    fun 远端核验期间本地内容变化时取消删除() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("keep.txt"), "keep")
        val (mtime, size) = stat(mount.resolve("keep.txt"))
        val engine = MockEngine { request ->
            val url = request.url.toString()
            when {
                url.contains("getStartCursor") ->
                    respond("""{"category":"drive#startCursor","startCursor":"s0"}""", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                url.contains("/changes?") ->
                    respond("""{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}""", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                request.method == HttpMethod.Get && url.contains("/files/f1") -> {
                    // 模拟远端核验窗口内的并发编辑
                    Files.writeString(mount.resolve("keep.txt"), "tampered-content")
                    respond("{}", HttpStatusCode.NotFound, headersOf(HttpHeaders.ContentType, "application/json"))
                }
                url.contains("/files?") ->
                    respond(remoteList(), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                else -> error("未预期请求: ${request.method.value} $url")
            }
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "keep.txt", localMtime = mtime, localSize = size)
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertTrue(Files.exists(mount.resolve("keep.txt")), "核验窗口内变化的本地内容必须保留")
            assertEquals("tampered-content", Files.readString(mount.resolve("keep.txt")))
            assertNotNull(db.syncItems.findByFileId("f1"))
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §4.4 成功路径回归：孤儿占位符（无基线、无云端）仍被正常清理。
     */
    @Test
    fun 孤儿占位符仍被正常清理() = runBlocking {
        val (_, mount, data) = workspace()
        JvmPlaceholderManager(mount, MacXattrAccess).createPlaceholderStrict("orphan.txt")
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                url.contains("/files?") -> remoteList()
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertFalse(Files.exists(mount.resolve("orphan.txt")), "孤儿占位符应被清理")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §4.5（原 results.rs:204-233 + cycle.rs:623-628）：移动+编辑时基线保留旧内容版本，
     * 并立即触发重扫补传内容差异。
     */
    @Test
    fun 移动加编辑保留旧内容基线并触发重扫补传() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("new.txt"), "edited")
        var renameCalls = 0
        var updateCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Get && url.contains("/files/f1") ->
                    remoteFile("f1", "old.txt", 3L, editedT1)
                request.method == HttpMethod.Patch && url.contains("uploadType=multipart") -> {
                    updateCalls++
                    remoteFile("f1", "new.txt", 6L, editedT2)
                }
                request.method == HttpMethod.Patch -> {
                    renameCalls++
                    remoteFile("f1", "new.txt", 3L, editedT1)
                }
                url.contains("/files?") -> remoteList(remoteFile("f1", "old.txt", 3L, editedT1))
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "old.txt", localMtime = 1_000L, localSize = 3L)
        db.inodeMap.upsert(
            PlatformInode.readInode(mount.resolve("new.txt").toString()),
            "old.txt",
            "f1",
            System.currentTimeMillis(),
        )
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertEquals(1, renameCalls, "移动应提交云端改名")
            val moved = assertNotNull(db.syncItems.findByFileId("f1"))
            assertEquals("new.txt", moved.localPath)
            // 若基线在移动结算时被当前 stat 重写（旧缺陷），编辑会被误判为已同步，
            // 后续周期不再产生补传；轮询补传发生即可同时证明「保留旧基线」与「立即重扫」。
            var settledSize = moved.localSize
            repeat(500) {
                if (settledSize == 6L) return@repeat
                delay(10)
                settledSize = db.syncItems.findByFileId("f1")?.localSize
            }
            assertEquals(1, updateCalls, "移动后应立即重扫并补传移动前后的编辑")
            assertEquals(6L, settledSize, "补传成功后基线记录新源快照")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §4.6（原 reconciliation.rs:712-758）：云端删除前 re-stat 本地路径，
     * 文件实际存在（非占位符）则降级取消。
     */
    @Test
    fun 本地文件实际存在时取消云端删除() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("note.skipme"), "real")
        var deleteCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Patch -> {
                    deleteCalls++
                    "{}"
                }
                url.contains("/files?") -> remoteList(remoteFile("f1", "note.skipme", 4L, editedT1))
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        // 扫描器按用户 skip 规则过滤本地条目，模拟「扫描认为缺失、实际存在」的删除场景
        val config = memoryConfig(
            UserConfig(mountDir = mount.toString(), mountConfigured = true, skipPatterns = listOf("*.skipme")),
        )
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "note.skipme", localMtime = 1L, localSize = 4L)
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertEquals(0, deleteCalls, "本地文件实际存在时禁止云端删除")
            assertNotNull(db.syncItems.findByFileId("f1"), "降级取消不得推进基线")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §4.7（原 actions.rs:365-378）：目标目录已存在同名云端文件时取消移动，拒绝覆盖。
     */
    @Test
    fun 目标目录同名云端文件存在时取消移动() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("new.txt"), "edited")
        var renameCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Patch -> {
                    renameCalls++
                    "{}"
                }
                url.contains("/files?") -> remoteList(
                    remoteFile("f1", "old.txt", 3L, editedT1),
                    remoteFile("g2", "new.txt", 6L, editedT1),
                )
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "f1", "old.txt", localMtime = 1_000L, localSize = 3L)
        db.inodeMap.upsert(
            PlatformInode.readInode(mount.resolve("new.txt").toString()),
            "old.txt",
            "f1",
            System.currentTimeMillis(),
        )
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertEquals(0, renameCalls, "目标目录撞名时禁止远端写入")
            assertEquals("old.txt", db.syncItems.findByFileId("f1")?.localPath, "取消移动不得推进基线")
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    /**
     * §4.8（原 transfer_operations.rs:380-381）：下载完成后即时记账 inode 映射，
     * 下轮刷新前改名也不丢身份。
     */
    @Test
    fun 下载完成后即时更新inode映射() = runBlocking {
        val (_, mount, data) = workspace()
        Files.writeString(mount.resolve("a.txt"), "v1old")
        val (mtime, size) = stat(mount.resolve("a.txt"))
        val engine = MockEngine { request ->
            val url = request.url.toString()
            when {
                url.contains("getStartCursor") ->
                    respond("""{"category":"drive#startCursor","startCursor":"s0"}""", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                url.contains("/changes?") ->
                    respond("""{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}""", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                url.contains("form=content") ->
                    respond("hello", HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "text/plain"))
                request.method == HttpMethod.Get && url.contains("/files/remote") ->
                    respond(remoteFile("remote", "a.txt", 5L, editedT2), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                url.contains("/files?") ->
                    respond(remoteList(remoteFile("remote", "a.txt", 5L, editedT2)), HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
                else -> error("未预期请求: ${request.method.value} $url")
            }
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = memoryConfig(UserConfig(mountDir = mount.toString(), mountConfigured = true))
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        seedBaseline(db, "remote", "a.txt", localMtime = mtime, localSize = size, cloudEditedTime = editedT1Ms)
        val runtime = newRuntime(paths, config, db, client)
        try {
            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertEquals("hello", Files.readString(mount.resolve("a.txt")))
            val inode = PlatformInode.readInode(mount.resolve("a.txt").toString())
            val record = assertNotNull(db.inodeMap.lookup(inode), "下载落地的新 inode 必须即时记账")
            assertEquals("remote", record.fileId)
            assertEquals("a.txt", record.relativePath)
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }
}
