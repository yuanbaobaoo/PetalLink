package io.github.yuanbaobaao.petallink.drive

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.auth.TokenPair
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue
import java.io.IOException

class UploadApiHttpTest {
    @Test
    fun multipart必须为related且完整核验id名称父目录和大小() { runBlocking {
        var contentType = ""
        val api = api(MockEngine { request ->
            contentType = request.body.contentType?.toString()
                ?: request.headers[HttpHeaders.ContentType].orEmpty()
            respond(
                """{"category":"drive#file","id":"f1","fileName":"花🌸.txt","mimeType":"text/plain","parentFolder":["p1"],"size":"3"}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val file = api.uploadSmall("花🌸.txt", "p1", byteArrayOf(1, 2, 3))
        assertEquals("f1", file.id)
        assertTrue(
            contentType.startsWith("multipart/related; boundary=hwcloud_"),
            "实际 Content-Type: $contentType",
        )
    } }

    @Test
    fun multipart写响应必须为HTTP200() { runBlocking {
        val api = api(MockEngine {
            respond(
                """{"id":"f1","fileName":"a","mimeType":"text/plain","size":1}""",
                HttpStatusCode.Created,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        assertFailsWith<AppError.Remote> { api.uploadSmall("a", null, byteArrayOf(1)) }
    } }

    @Test
    fun resume初始化必须使用Location并校验分片大小() { runBlocking {
        val api = api(MockEngine {
            respond(
                """{"sliceSize":10485760}""",
                HttpStatusCode.OK,
                headersOf(
                    HttpHeaders.ContentType to listOf("application/json"),
                    HttpHeaders.Location to listOf("https://upload.test/session?uploadId=u1"),
                ),
            )
        })
        val session = api.initResume("large.bin", "p1", 30L * 1024 * 1024)
        assertEquals("https://upload.test/session?uploadId=u1", session.requestUrl())
        assertEquals(10L * 1024 * 1024, session.chunkSize)
    } }

    @Test
    fun resume的308状态只使用服务端rangeList确认偏移() { runBlocking {
        val api = api(MockEngine {
            respond(
                """{"rangeList":["0-2","3-5"]}""",
                HttpStatusCode.PermanentRedirect,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val status = api.querySessionStatus(
            ResumeSession("", "", "https://upload.test/session", 1024 * 1024),
            totalSize = 10,
        )
        assertEquals(6, status.uploaded)
        assertEquals(null, status.finalFile)
    } }

    @Test
    fun update必须PATCH指定fileId且绝不创建新文件() { runBlocking {
        var method = ""
        var path = ""
        val api = api(MockEngine { request ->
            method = request.method.value
            path = request.url.encodedPath
            respond(
                """{"category":"drive#file","id":"f1","fileName":"a.txt","mimeType":"text/plain","size":"2"}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val file = api.uploadSmallUpdate("f1", "a.txt", null, byteArrayOf(1, 2))
        assertEquals("f1", file.id)
        assertEquals("PATCH", method)
        assertTrue(path.endsWith("/files/f1"))
    } }

    @Test
    fun putChunk只采用308服务端确认偏移() { runBlocking {
        var contentRange = ""
        val api = api(MockEngine { request ->
            contentRange = request.headers[HttpHeaders.ContentRange].orEmpty()
            respond(
                """{"rangeList":["0-3"]}""",
                HttpStatusCode.PermanentRedirect,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val result = api.putChunk(
            ResumeSession("", "", "https://upload.test/session", 4),
            offset = 0,
            totalSize = 10,
            content = byteArrayOf(1, 2, 3, 4),
        )
        assertEquals(4L, result.uploaded)
        assertEquals("bytes 0-3/10", contentRange)
    } }

    @Test
    fun 分片响应丢失后查询同一会话而不按本地长度推算() { runBlocking {
        var calls = 0
        val api = api(MockEngine {
            calls++
            if (calls == 1) throw IOException("lost response")
            respond(
                """{"rangeList":["0-1"]}""",
                HttpStatusCode.PermanentRedirect,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val result = api.putChunk(
            ResumeSession("", "", "https://upload.test/session", 4),
            offset = 0,
            totalSize = 10,
            content = byteArrayOf(1, 2, 3, 4),
        )
        assertEquals(2L, result.uploaded)
        assertEquals(2, calls)
    } }

    private fun api(engine: MockEngine): UploadApi = UploadApi(
        DriveClient(
            HttpClient(engine),
            tokenProvider = { "token" },
            tokenRefresher = { TokenPair("new", "refresh", Long.MAX_VALUE) },
        ),
        "https://example.test/upload/drive/v1",
    )
}
