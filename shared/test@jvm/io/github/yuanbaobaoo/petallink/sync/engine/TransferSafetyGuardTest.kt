package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.auth.TokenPair
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.drive.DownloadApi
import io.github.yuanbaobaoo.petallink.drive.DriveClient
import io.github.yuanbaobaoo.petallink.drive.UploadApi
import io.github.yuanbaobaoo.petallink.drive.UploadProtocol
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import java.io.IOException
import java.nio.file.Files
import java.time.Instant
import kotlin.io.path.createTempDirectory
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

/**
 * 传输层安全护栏集成测试（§2.1/§3.1/§3.3/§3.5/§3.6/§3.7/§3.11，对照 Rust 原版移植语义）。
 */
class TransferSafetyGuardTest {
    // ------------------------------------------------------------------
    // §3.5 Update 上传前 re-GET 远端元数据比对 editedTime
    // ------------------------------------------------------------------

    @Test
    fun 更新上传远端版本已变化拒绝用旧任务覆盖() = withEnvironment { dir, db ->
        val source = dir.resolve("up.bin")
        Files.write(source, "hello".encodeToByteArray())
        val store = JvmTransferFileStore()
        val snapshot = store.snapshot(source.toString())
        val methods = mutableListOf<String>()
        val operations = operations(store, MockEngine { request ->
            methods += request.method.value
            respond(
                """{"id":"f1","fileName":"up.bin","mimeType":"application/octet-stream","size":"5","editedTime":"2026-07-17T00:00:00Z"}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, 5L).copy(
                operation = 1,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
                expectedCloudEditedTime = Instant.parse("2026-07-16T00:00:00Z").toEpochMilli(),
            ),
        )

        assertEquals(TaskDisposition.RESTART_REQUIRED, runner(db, operations).runExpected(context(db, id)))
        assertEquals(TransferState.RestartRequired, db.transfers.findById(id)?.state)
        assertFalse("PATCH" in methods, "远端版本不一致时不允许发出任何写入")
    }

    @Test
    fun 更新上传远端版本一致放行并完成() = withEnvironment { dir, db ->
        val source = dir.resolve("up.bin")
        Files.write(source, "hello".encodeToByteArray())
        val store = JvmTransferFileStore()
        val snapshot = store.snapshot(source.toString())
        val operations = operations(store, MockEngine { request ->
            if (request.method.value == "PATCH") {
                respond(
                    """{"category":"drive#file","id":"f1","fileName":"up.bin","mimeType":"application/octet-stream","size":"5","editedTime":"2026-07-16T00:00:00Z"}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            } else {
                respond(
                    """{"id":"f1","fileName":"up.bin","mimeType":"application/octet-stream","size":"5","editedTime":"2026-07-16T00:00:00Z"}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            }
        })
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, 5L).copy(
                operation = 1,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
                expectedCloudEditedTime = Instant.parse("2026-07-16T00:00:00Z").toEpochMilli(),
            ),
        )

        assertEquals(TaskDisposition.COMPLETED, runner(db, operations).runExpected(context(db, id)))
    }

    // ------------------------------------------------------------------
    // §3.6 上传响应缺 editedTime 时补取完整元数据
    // ------------------------------------------------------------------

    @Test
    fun 上传响应缺编辑时间补取元数据成功后完成() = withEnvironment { dir, db ->
        val source = dir.resolve("up.bin")
        Files.write(source, "hello".encodeToByteArray())
        val store = JvmTransferFileStore()
        val snapshot = store.snapshot(source.toString())
        var metadataGets = 0
        val operations = operations(store, MockEngine { request ->
            when {
                request.method.value == "GET" && request.url.parameters["queryParam"] != null ->
                    respond(
                        """{"category":"drive#fileList","files":[]}""",
                        HttpStatusCode.OK,
                        headersOf(HttpHeaders.ContentType, "application/json"),
                    )
                request.method.value == "GET" -> {
                    metadataGets++
                    respond(
                        """{"id":"new-id","fileName":"up.bin","mimeType":"application/octet-stream","size":"5","editedTime":"2026-07-16T00:00:00Z"}""",
                        HttpStatusCode.OK,
                        headersOf(HttpHeaders.ContentType, "application/json"),
                    )
                }
                else -> respond(
                    """{"category":"drive#file","id":"new-id","fileName":"up.bin","mimeType":"application/octet-stream","size":"5"}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            }
        })
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, 5L).copy(
                fileId = null,
                operation = 0,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
            ),
        )

        assertEquals(TaskDisposition.COMPLETED, runner(db, operations).runExpected(context(db, id)))
        assertEquals(1, metadataGets, "缺 editedTime 时必须补取一次完整元数据")
        assertEquals("new-id", db.transfers.findById(id)?.remoteResultFileId)
    }

    @Test
    fun 上传响应缺编辑时间补取失败转远端核验() = withEnvironment { dir, db ->
        val source = dir.resolve("up.bin")
        Files.write(source, "hello".encodeToByteArray())
        val store = JvmTransferFileStore()
        val snapshot = store.snapshot(source.toString())
        val operations = operations(store, MockEngine { request ->
            when {
                request.method.value == "GET" && request.url.parameters["queryParam"] != null ->
                    respond(
                        """{"category":"drive#fileList","files":[]}""",
                        HttpStatusCode.OK,
                        headersOf(HttpHeaders.ContentType, "application/json"),
                    )
                request.method.value == "GET" ->
                    respond(ByteArray(0), HttpStatusCode.InternalServerError)
                else -> respond(
                    """{"category":"drive#file","id":"new-id","fileName":"up.bin","mimeType":"application/octet-stream","size":"5"}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            }
        })
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, 5L).copy(
                fileId = null,
                operation = 0,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
            ),
        )

        assertEquals(TaskDisposition.VERIFYING_REMOTE, runner(db, operations).runExpected(context(db, id)))
        val task = assertNotNull(db.transfers.findById(id))
        assertEquals(TransferState.VerifyingRemote, task.state)
        assertEquals("new-id", task.remoteResultFileId)
    }

    // ------------------------------------------------------------------
    // §3.7 Create 上传前列父目录同名预检
    // ------------------------------------------------------------------

    @Test
    fun 创建上传撞名拒绝重复创建() = withEnvironment { dir, db ->
        val source = dir.resolve("up.bin")
        Files.write(source, "hello".encodeToByteArray())
        val store = JvmTransferFileStore()
        val snapshot = store.snapshot(source.toString())
        val methods = mutableListOf<String>()
        val operations = operations(store, MockEngine { request ->
            methods += request.method.value
            respond(
                """{"category":"drive#fileList","files":[{"id":"x1","fileName":"up.bin","mimeType":"application/octet-stream","size":"5"}]}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, 5L).copy(
                fileId = null,
                operation = 0,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
            ),
        )

        assertEquals(TaskDisposition.RESTART_REQUIRED, runner(db, operations).runExpected(context(db, id)))
        assertFalse("POST" in methods, "撞名时不允许发出创建请求")
    }

    // ------------------------------------------------------------------
    // §3.3 上传/下载错误分类
    // ------------------------------------------------------------------

    @Test
    fun 上传网络错误可能已送达转远端核验而非盲目重放() = withEnvironment { dir, db ->
        val source = dir.resolve("up.bin")
        Files.write(source, "hello".encodeToByteArray())
        val store = JvmTransferFileStore()
        val snapshot = store.snapshot(source.toString())
        val operations = operations(store, MockEngine { request ->
            if (request.method.value == "GET") {
                respond(
                    """{"category":"drive#fileList","files":[]}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            } else {
                throw IOException("broken pipe")
            }
        })
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, 5L).copy(
                fileId = null,
                operation = 0,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
            ),
        )

        assertEquals(TaskDisposition.VERIFYING_REMOTE, runner(db, operations).runExpected(context(db, id)))
        assertEquals(TransferState.VerifyingRemote, db.transfers.findById(id)?.state)
    }

    @Test
    fun 下载网络错误等待网络而不进远端核验() = withEnvironment { dir, db ->
        val destination = dir.resolve("down.bin").toString()
        val store = JvmTransferFileStore()
        val operations = operations(store, MockEngine { throw IOException("connection reset") })
        val id = db.transfers.insert(task(destination, TransferDirection.DOWNLOAD, 6L))

        assertEquals(TaskDisposition.WAITING_FOR_NETWORK, runner(db, operations).runExpected(context(db, id)))
        assertEquals(TransferState.WaitingForNetwork, db.transfers.findById(id)?.state)
    }

    @Test
    fun 续传会话失效转远端核验() = withEnvironment { dir, db ->
        val source = dir.resolve("large.bin")
        val total = UploadProtocol.SMALL_LARGE_THRESHOLD + 1L
        Files.newOutputStream(source).use { output ->
            val block = ByteArray(1024 * 1024) { 7 }
            repeat(20) { output.write(block) }
            output.write(7)
        }
        val store = JvmTransferFileStore()
        val snapshot = store.snapshot(source.toString())
        val operations = operations(store, MockEngine { request ->
            if (request.method.value == "GET") {
                respond(
                    """{"category":"drive#fileList","files":[]}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            } else {
                respond("""{"error":{"code":"CONTENT_NOT_FOUND"}}""", HttpStatusCode.NotFound)
            }
        })
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, total).copy(
                fileId = null,
                parentFileId = "root",
                operation = 0,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
                sessionUrl = "https://upload.test/session",
                serverId = "s1",
                uploadId = "u1",
            ),
        )

        // upload_session_expired（404 + CONTENT_NOT_FOUND）→ 只能核验远端，禁止新建会话盲重放
        assertEquals(TaskDisposition.VERIFYING_REMOTE, runner(db, operations).runExpected(context(db, id)))
        assertEquals(TransferState.VerifyingRemote, db.transfers.findById(id)?.state)
    }

    // ------------------------------------------------------------------
    // §3.1 + §2.1 下载目标身份/快照护栏
    // ------------------------------------------------------------------

    @Test
    fun 下载期间本地目标被修改保留用户内容并转重新规划() = withEnvironment { dir, db ->
        val destination = dir.resolve("x.bin")
        Files.write(destination, "user-content".encodeToByteArray())
        val store = JvmTransferFileStore()
        val probes = mutableListOf(
            DownloadTargetIdentity.Occupied(12L, 1_000L),   // preflight
            DownloadTargetIdentity.Occupied(12L, 1_000L),   // 执行期捕获快照
            DownloadTargetIdentity.Occupied(13L, 1_001L),   // 安装前复核：已被改动
        )
        var probeCalls = 0
        val operations = operations(
            store,
            downloadMock("f1", "abcdef"),
            probe = { probes[probeCalls++.coerceAtMost(probes.lastIndex)] },
        )
        val id = db.transfers.insert(task(destination.toString(), TransferDirection.DOWNLOAD, 6L))

        assertEquals(TaskDisposition.RESTART_REQUIRED, runner(db, operations).runExpected(context(db, id)))
        assertEquals(TransferState.RestartRequired, db.transfers.findById(id)?.state)
        assertEquals("user-content", Files.readString(destination), "用户内容必须保留")
        assertTrue(Files.exists(dir.resolve("x.bin.tmp")), "下载临时文件必须保留")
    }

    @Test
    fun 下载安装前占位身份不属于同一云端文件拒绝覆盖() = withEnvironment { dir, db ->
        val destination = dir.resolve("x.bin")
        Files.write(destination, ByteArray(0))
        val store = JvmTransferFileStore()
        val operations = operations(
            store,
            downloadMock("f1", "abcdef"),
            probe = { DownloadTargetIdentity.Placeholder(fileId = "someone-else") },
        )
        val id = db.transfers.insert(task(destination.toString(), TransferDirection.DOWNLOAD, 6L))

        assertEquals(TaskDisposition.RESTART_REQUIRED, runner(db, operations).runExpected(context(db, id)))
        assertEquals(0L, Files.size(destination), "身份不符的占位符不允许被覆盖安装")
    }

    @Test
    fun 下载执行前备份修改过的占位符且目标空缺正常安装() = withEnvironment { dir, db ->
        val destination = dir.resolve("x.bin")
        val store = JvmTransferFileStore()
        val backups = mutableListOf<String>()
        val operations = operations(
            store,
            downloadMock("f1", "abcdef"),
            probe = { DownloadTargetIdentity.Missing },
            backup = { backups += it },
        )
        val id = db.transfers.insert(task(destination.toString(), TransferDirection.DOWNLOAD, 6L))

        assertEquals(TaskDisposition.COMPLETED, runner(db, operations).runExpected(context(db, id)))
        assertEquals(listOf(destination.toString()), backups, "下载执行前必须调用占位符备份")
        assertEquals("abcdef", Files.readString(destination))
    }

    // ------------------------------------------------------------------
    // §3.11 429 优先服务端 Retry-After
    // ------------------------------------------------------------------

    @Test
    fun 四二九退避优先采用服务端RetryAfter() = withEnvironment { dir, db ->
        val source = dir.resolve("up.bin")
        Files.write(source, "hello".encodeToByteArray())
        val store = JvmTransferFileStore()
        val snapshot = store.snapshot(source.toString())
        val operations = operations(store, MockEngine { request ->
            if (request.method.value == "GET") {
                respond(
                    """{"category":"drive#fileList","files":[]}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            } else {
                respond(
                    ByteArray(0),
                    HttpStatusCode.TooManyRequests,
                    headersOf(HttpHeaders.RetryAfter, "120"),
                )
            }
        })
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, 5L).copy(
                fileId = null,
                operation = 0,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
            ),
        )

        assertEquals(TaskDisposition.BACKING_OFF, runner(db, operations).runExpected(context(db, id)))
        // runner 时钟 10_000 + Retry-After 120s，而非本地指数退避 10_000+1_000
        assertEquals(130_000L, db.transfers.findById(id)?.nextRetryAt)
    }

    // ------------------------------------------------------------------
    // 测试基建（同 PersistentTransferIntegrationTest 风格）
    // ------------------------------------------------------------------

    private fun downloadMock(fileId: String, content: String) = MockEngine { request ->
        if (request.url.parameters["form"] == "content") {
            respond(content.encodeToByteArray(), HttpStatusCode.OK)
        } else {
            respond(
                """{"id":"$fileId","fileName":"x.bin","mimeType":"application/octet-stream","size":"${content.length}","editedTime":"v1"}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType to listOf("application/json"), HttpHeaders.ETag to listOf("etag-1")),
            )
        }
    }

    private fun operations(
        store: TransferFileStore,
        engine: MockEngine,
        probe: (suspend (String) -> DownloadTargetIdentity)? = null,
        backup: suspend (String) -> Unit = {},
    ): TransferOperationsImpl {
        val drive = DriveClient(
            HttpClient(engine),
            tokenProvider = { "token" },
            tokenRefresher = { TokenPair("new", "refresh", Long.MAX_VALUE) },
        )
        return TransferOperationsImpl(
            uploadApi = UploadApi(drive, "https://example.test/upload/drive/v1"),
            downloadApi = DownloadApi(drive, "https://example.test/drive/v1"),
            readFileBytes = { error("legacy read") },
            writeFileBytes = { _, _ -> error("legacy write") },
            fileExists = store::exists,
            fileSize = store::size,
            fileStore = store,
            downloadTargetProbe = probe,
            backupModifiedPlaceholder = backup,
        )
    }

    private fun runner(db: PetalLinkDb, operations: TransferOperations) =
        TaskRunner(db.transfers, operations, { true }, { 10_000L }, { 0L })

    private fun task(path: String, direction: TransferDirection, size: Long) = TransferTask(
        id = null,
        direction = direction,
        fileId = "f1",
        localPath = path,
        name = path.substringAfterLast('/'),
        totalSize = size,
        state = TransferState.Pending,
        errorMessage = null,
        createdAt = 1L,
        operation = if (direction == TransferDirection.UPLOAD) 0 else 2,
    )

    private suspend fun context(db: PetalLinkDb, id: Long): TaskContext {
        val task = db.transfers.findById(id)!!
        return TaskContext(
            id, task.fileId.orEmpty(), task.localPath.orEmpty(), task.direction, task.state,
            task.stateRevision, task.attempt, task.bytesTotal, task.bytesDone,
            task.nextRetryAt, task.remoteResultFileId, task.sessionUrl, task.serverId, task.uploadId,
            task.parentFileId, task.operation, task.sourceMtime, task.sourceSize, task.expectedCloudEditedTime,
        )
    }

    private fun withEnvironment(block: suspend (java.nio.file.Path, PetalLinkDb) -> Unit) = runBlocking {
        val dir = createTempDirectory("petallink-guard-")
        val db = PetalLinkDb(dir.resolve("state.db").toString())
        try {
            block(dir, db)
        } finally {
            db.close()
            dir.toFile().deleteRecursively()
        }
    }
}
