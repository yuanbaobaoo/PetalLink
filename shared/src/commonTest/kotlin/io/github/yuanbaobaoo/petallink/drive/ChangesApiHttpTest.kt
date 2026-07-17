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
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class ChangesApiHttpTest {
    @Test
    fun 过期游标产生专用全量回退错误() { runBlocking {
        val api = api(MockEngine { respond("expired", HttpStatusCode.Gone) })
        val error = assertFailsWith<AppError.ChangesCursorInvalid> {
            api.listChanges("old-cursor")
        }
        assertEquals(410, error.status)
    } }

    @Test
    fun 空中间页仍继续且只提交终页newStartCursor() { runBlocking {
        var calls = 0
        val api = api(MockEngine {
            calls++
            val body = if (calls == 1) {
                """{"category":"drive#changeList","changes":[],"nextCursor":"next"}"""
            } else {
                """{"category":"drive#changeList","changes":[],"newStartCursor":"checkpoint"}"""
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        })
        val (changes, cursor) = api.listAllChanges("start")
        assertEquals(emptyList(), changes)
        assertEquals("checkpoint", cursor)
        assertEquals(2, calls)
    } }

    @Test
    fun 无变更单页允许newStartCursor与请求cursor相同() { runBlocking {
        val api = api(MockEngine {
            respond(
                """{"category":"drive#changeList","changes":[],"newStartCursor":"same"}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        })
        val result = api.listAllChanges("same")
        assertEquals("same", result.second)
    } }

    private fun api(engine: MockEngine): ChangesApi = ChangesApi(
        DriveClient(
            HttpClient(engine),
            tokenProvider = { "token" },
            tokenRefresher = { TokenPair("new", "refresh", Long.MAX_VALUE) },
        ),
        "https://example.test/drive/v1",
    )
}
