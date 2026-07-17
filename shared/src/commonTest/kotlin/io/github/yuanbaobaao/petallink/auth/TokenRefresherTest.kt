package io.github.yuanbaobaao.petallink.auth

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class TokenRefresherTest {
    @Test
    fun 并发refresh只发送一次请求且只保存一次() { runBlocking {
        val requests = AtomicInteger()
        val saves = AtomicInteger()
        val engine = MockEngine {
            requests.incrementAndGet()
            delay(50)
            respond(
                """{"access_token":"fresh","refresh_token":"next","expires_in":3600}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }
        val current = TokenPair("old", "refresh", Long.MAX_VALUE)
        val refresher = TokenRefresher(
            HttpClient(engine),
            clientIdProvider = { "client" },
            clientSecretProvider = { "secret" },
            currentTokenProvider = { current },
            onTokenSaved = { saves.incrementAndGet() },
        )
        val results = List(20) { async { refresher.refresh() } }.awaitAll()
        assertEquals(1, requests.get())
        assertEquals(1, saves.get())
        assertEquals(setOf("fresh"), results.map { it.accessToken }.toSet())
    } }

    @Test
    fun 刷新响应缺少refreshToken时沿用旧值() { runBlocking {
        val engine = MockEngine {
            respond(
                """{"access_token":"fresh","expires_in":3600}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }
        val current = TokenPair("old", "keep-me", Long.MAX_VALUE)
        val result = TokenRefresher(
            HttpClient(engine), { "client" }, { "secret" }, { current }, {},
        ).refresh()
        assertEquals("keep-me", result.refreshToken)
    } }
}
