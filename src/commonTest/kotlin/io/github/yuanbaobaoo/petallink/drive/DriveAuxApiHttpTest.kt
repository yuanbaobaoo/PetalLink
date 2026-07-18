package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.auth.TokenPair
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class DriveAuxApiHttpTest {
    @Test
    fun about强制fields并容忍字符串与小数配额() { runBlocking {
        var url = ""
        val api = AboutApi(client(MockEngine { request ->
            url = request.url.toString()
            respond(
                """{"storageQuota":{"userCapacity":"1000","usedSpace":12.9,"recycledSpace":"3"}}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }), "https://example.test/drive/v1")
        val quota = api.getQuota()
        assertTrue(url.endsWith("/about?fields=*"))
        assertEquals(1000, quota.totalBytes())
        assertEquals(12, quota.usedBytes())
        assertEquals(3, tolerantLong(quota.recycled))
        api.ensureUploadCapacity(988)
        assertFailsWith<AppError.Data> { api.ensureUploadCapacity(989) }
    } }

    @Test
    fun thumbnail使用真实端点并返回原始字节() { runBlocking {
        var url = ""
        val api = ThumbnailApi(client(MockEngine { request ->
            url = request.url.toString()
            respond(byteArrayOf(0, 1, 0xff.toByte()), HttpStatusCode.OK)
        }), "https://example.test/drive/v1")
        assertContentEquals(byteArrayOf(0, 1, 0xff.toByte()), api.getThumbnail("id/a"))
        assertTrue(url.contains("/thumbnails/id%2Fa?form=content"))
    } }

    @Test
    fun download元数据非200不解析为成功() { runBlocking {
        val api = DownloadApi(client(MockEngine {
            respond("{}", HttpStatusCode.NotFound)
        }), "https://example.test/drive/v1")
        assertFailsWith<AppError.Remote> { api.fetchRemoteMetadata("missing") }
    } }

    private fun client(engine: MockEngine) = DriveClient(
        HttpClient(engine),
        tokenProvider = { "token" },
        tokenRefresher = { TokenPair("new", "refresh", Long.MAX_VALUE) },
    )
}
