package io.github.yuanbaobaao.petallink.drive

import io.github.yuanbaobaao.petallink.auth.TokenPair
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpMethod
import io.ktor.http.HttpStatusCode
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class DriveClientHttpTest {
    @Test
    fun 收到401只刷新并重放一次() { runBlocking {
        val requests = AtomicInteger()
        val refreshes = AtomicInteger()
        val authorizations = mutableListOf<String?>()
        val engine = MockEngine { request ->
            authorizations += request.headers[HttpHeaders.Authorization]
            val attempt = requests.incrementAndGet()
            respond(if (attempt == 1) "unauthorized" else "ok", if (attempt == 1) HttpStatusCode.Unauthorized else HttpStatusCode.OK)
        }
        val client = DriveClient(
            HttpClient(engine),
            tokenProvider = { "old" },
            tokenRefresher = {
                refreshes.incrementAndGet()
                TokenPair("fresh", "refresh", Long.MAX_VALUE)
            },
        )
        val response = client.executeWithRetry(HttpMethod.Get, "https://example.test/files", HttpSemantics.READ)
        assertEquals(HttpStatusCode.OK, response.status)
        assertEquals(2, requests.get())
        assertEquals(1, refreshes.get())
        assertEquals(listOf<String?>("Bearer old", "Bearer fresh"), authorizations)
    } }
}
