package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.auth.TokenPair
import io.github.yuanbaobaao.petallink.data.TransferDirection
import io.github.yuanbaobaao.petallink.drive.DriveClient
import io.github.yuanbaobaao.petallink.drive.FilesApi
import io.github.yuanbaobaao.petallink.sync.TransferState
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import java.nio.file.Files
import java.time.Instant
import kotlin.io.path.createTempDirectory
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs

class JvmRemoteTransferVerifierTest {
    @Test
    fun create查不到或查到多个都保持Ambiguous禁止重放() = withSource { source, store ->
        for (files in listOf("", "${fileJson("a")},${fileJson("b")}")) {
            val verifier = verifier(store, MockEngine {
                respond(
                    """{"category":"drive#fileList","files":[$files]}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            })
            assertIs<RemoteVerification.Ambiguous>(verifier.verify(createTask(source.toString())))
        }
    }

    @Test
    fun create仅在身份时间窗和contentHash唯一匹配时确认() = withSource { source, store ->
        val hash = store.sha256(source.toString())
        val verifier = verifier(store, MockEngine {
            respond(
                """{"category":"drive#fileList","files":[${fileJson("created", hash)}]}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val result = assertIs<RemoteVerification.Committed>(verifier.verify(createTask(source.toString())))
        assertEquals("created", result.fileId)
    }

    @Test
    fun update只核验原fileId且editedTime未变时判定未提交() = withSource { source, store ->
        val unchanged = verifier(store, MockEngine {
            respond(fileJson("f1", editedTime = "2026-07-16T00:00:00Z"), HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"))
        })
        val task = createTask(source.toString()).copy(
            fileId = "f1",
            operation = 1,
            expectedCloudEditedTime = Instant.parse("2026-07-16T00:00:00Z").toEpochMilli(),
        )
        assertIs<RemoteVerification.NotCommitted>(unchanged.verify(task))

        val changed = verifier(store, MockEngine {
            respond(fileJson("f1", editedTime = "2026-07-16T00:00:01Z"), HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"))
        })
        assertEquals("f1", assertIs<RemoteVerification.Committed>(changed.verify(task)).fileId)
    }

    private fun createTask(path: String) = TaskContext(
        id = 1,
        fileId = "",
        localPath = path,
        direction = TransferDirection.UPLOAD,
        state = TransferState.VerifyingRemote,
        stateRevision = 1,
        attempt = 0,
        bytesTotal = 3,
        bytesDone = 0,
        parentFileId = "root",
        operation = 0,
        createdAt = Instant.parse("2026-07-16T00:00:00Z").toEpochMilli(),
    )

    private fun fileJson(
        id: String,
        hash: String? = null,
        editedTime: String = "2026-07-16T00:00:01Z",
    ): String = """{"category":"drive#file","id":"$id","fileName":"source.bin","mimeType":"application/octet-stream","parentFolder":["root"],"size":"3","createdTime":"2026-07-16T00:00:01Z","editedTime":"$editedTime"${hash?.let { ",\"contentHash\":\"$it\"" }.orEmpty()}}"""

    private fun verifier(store: TransferFileStore, engine: MockEngine): JvmRemoteTransferVerifier {
        val client = DriveClient(
            HttpClient(engine), { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) },
        )
        return JvmRemoteTransferVerifier(FilesApi(client, "https://example.test/drive/v1"), store)
    }

    private fun withSource(block: suspend (java.nio.file.Path, JvmTransferFileStore) -> Unit) = runBlocking {
        val dir = createTempDirectory("petallink-remote-verify-")
        val source = dir.resolve("source.bin")
        Files.write(source, byteArrayOf(1, 2, 3))
        try {
            block(source, JvmTransferFileStore())
        } finally {
            dir.toFile().deleteRecursively()
        }
    }
}
