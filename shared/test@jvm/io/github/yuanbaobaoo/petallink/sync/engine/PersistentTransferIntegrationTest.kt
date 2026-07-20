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
import java.nio.file.Files
import kotlin.io.path.createTempDirectory
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class PersistentTransferIntegrationTest {
    @Test
    fun 下载从sidecar和tmp恢复并在二次元数据核验后原子安装() = withEnvironment { dir, db ->
        val destination = dir.resolve("download.bin").toString()
        val store = JvmTransferFileStore()
        val identity = DownloadResumeMetadata("f1", 6L, "v1", "etag-1", null)
        store.writeTemp(destination, 0, "abc".encodeToByteArray(), truncate = true)
        store.writeResumeMetadata(destination, identity)
        var contentRequests = 0
        val operations = operations(store, MockEngine { request ->
            if (request.url.parameters["form"] == "content") {
                contentRequests++
                assertEquals("bytes=3-", request.headers[HttpHeaders.Range])
                respond(
                    "def".encodeToByteArray(),
                    HttpStatusCode.PartialContent,
                    headersOf(HttpHeaders.ContentRange, "bytes 3-5/6"),
                )
            } else {
                respond(metadata("f1", 6, "v1"), HttpStatusCode.OK, metadataHeaders("etag-1"))
            }
        })
        val id = db.transfers.insert(task(destination, TransferDirection.DOWNLOAD, 6L))
        val result = runner(db, operations).runExpected(context(db, id))

        assertEquals(TaskDisposition.COMPLETED, result)
        assertEquals("abcdef", Files.readString(dir.resolve("download.bin")))
        assertEquals(1, contentRequests)
        assertFalse(Files.exists(dir.resolve("download.bin.tmp")))
        assertFalse(Files.exists(dir.resolve("download.bin.download-meta.tmp")))
    }

    @Test
    fun Range416只允许一次清空断点后从零重启() = withEnvironment { dir, db ->
        val destination = dir.resolve("retry.bin").toString()
        val store = JvmTransferFileStore()
        store.writeTemp(destination, 0, "abc".encodeToByteArray(), truncate = true)
        store.writeResumeMetadata(destination, DownloadResumeMetadata("f1", 6L, "v1", "etag-1", null))
        var contentRequests = 0
        val operations = operations(store, MockEngine { request ->
            if (request.url.parameters["form"] == "content") {
                contentRequests++
                if (contentRequests == 1) {
                    assertEquals("bytes=3-", request.headers[HttpHeaders.Range])
                    respond(ByteArray(0), HttpStatusCode.RequestedRangeNotSatisfiable)
                } else {
                    assertEquals(null, request.headers[HttpHeaders.Range])
                    respond("abcdef".encodeToByteArray(), HttpStatusCode.OK)
                }
            } else {
                respond(metadata("f1", 6, "v1"), HttpStatusCode.OK, metadataHeaders("etag-1"))
            }
        })
        val id = db.transfers.insert(task(destination, TransferDirection.DOWNLOAD, 6L))

        assertEquals(TaskDisposition.COMPLETED, runner(db, operations).runExpected(context(db, id)))
        assertEquals(2, contentRequests)
        assertEquals("abcdef", Files.readString(dir.resolve("retry.bin")))
    }

    @Test
    fun 下载暂态错误保留断点和sidecar() = withEnvironment { dir, db ->
        val destination = dir.resolve("offline.bin").toString()
        val store = JvmTransferFileStore()
        val operations = operations(store, MockEngine { request ->
            if (request.url.parameters["form"] == "content") {
                respond(ByteArray(0), HttpStatusCode.InternalServerError)
            } else {
                respond(metadata("f1", 6, "v1"), HttpStatusCode.OK, metadataHeaders("etag-1"))
            }
        })
        val id = db.transfers.insert(task(destination, TransferDirection.DOWNLOAD, 6L))

        assertEquals(TaskDisposition.BACKING_OFF, runner(db, operations).runExpected(context(db, id)))
        assertTrue(Files.exists(dir.resolve("offline.bin.download-meta.tmp")))
    }

    @Test
    fun 大文件Create按服务端rangeList推进并持久化resume() = withEnvironment { dir, db ->
        val source = dir.resolve("large.bin")
        val total = UploadProtocol.SMALL_LARGE_THRESHOLD + 1L
        Files.newOutputStream(source).use { output ->
            val block = ByteArray(1024 * 1024) { 7 }
            repeat(20) { output.write(block) }
            output.write(7)
        }
        var put = 0
        val ranges = mutableListOf<String>()
        val store = JvmTransferFileStore()
        val operations = operations(store, MockEngine { request ->
            when {
                // §3.7 Create 上传前同名预检（列父目录）
                request.method.value == "GET" -> respond(
                    """{"category":"drive#fileList","files":[]}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
                request.method.value == "POST" -> {
                    respond(
                        """{"serverId":"s1","uploadId":"u1","sliceSize":10485760}""",
                        HttpStatusCode.OK,
                        headersOf(HttpHeaders.Location, "https://upload.test/session"),
                    )
                }
                else -> {
                    put++
                    ranges += request.headers[HttpHeaders.ContentRange].orEmpty()
                    when (put) {
                        1 -> respond("""{"rangeList":["0-10485759"]}""", HttpStatusCode.PermanentRedirect)
                        2 -> respond("""{"rangeList":["0-20971519"]}""", HttpStatusCode.PermanentRedirect)
                        else -> respond(
                            """{"category":"drive#file","id":"cloud-large","fileName":"large.bin","mimeType":"application/octet-stream","size":"$total","editedTime":"2026-07-16T00:00:00Z"}""",
                            HttpStatusCode.OK,
                            headersOf(HttpHeaders.ContentType, "application/json"),
                        )
                    }
                }
            }
        })
        val snapshot = store.snapshot(source.toString())
        val id = db.transfers.insert(
            task(source.toString(), TransferDirection.UPLOAD, total).copy(
                fileId = null,
                parentFileId = "root",
                operation = 0,
                sourceSize = snapshot.size,
                sourceMtime = snapshot.modifiedAtMillis,
            ),
        )

        assertEquals(TaskDisposition.COMPLETED, runner(db, operations).runExpected(context(db, id)))
        assertEquals(
            listOf(
                "bytes 0-10485759/$total",
                "bytes 10485760-20971519/$total",
                "bytes 20971520-20971520/$total",
            ),
            ranges,
        )
        val completed = db.transfers.findById(id)!!
        assertEquals(total, completed.resumeOffset)
        assertEquals("https://upload.test/session", completed.sessionUrl)
    }

    private fun operations(store: TransferFileStore, engine: MockEngine): TransferOperationsImpl {
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

    private fun metadata(id: String, size: Long, editedTime: String) =
        """{"id":"$id","fileName":"download.bin","mimeType":"application/octet-stream","size":"$size","editedTime":"$editedTime"}"""

    private fun metadataHeaders(etag: String) = headersOf(
        HttpHeaders.ContentType to listOf("application/json"),
        HttpHeaders.ETag to listOf(etag),
    )

    private fun withEnvironment(block: suspend (java.nio.file.Path, PetalLinkDb) -> Unit) = runBlocking {
        val dir = createTempDirectory("petallink-transfer-")
        val db = PetalLinkDb(dir.resolve("state.db").toString())
        try {
            block(dir, db)
        } finally {
            db.close()
            dir.toFile().deleteRecursively()
        }
    }
}
