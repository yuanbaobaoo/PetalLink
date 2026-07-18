package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.auth.TokenPair
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
import kotlin.test.assertFailsWith

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

    @Test
    fun 请求层异常统一分类为网络错误并只报告一次() = runBlocking {
        val failures = AtomicInteger()
        val client = DriveClient(
            HttpClient(MockEngine { error("socket closed") }),
            tokenProvider = { "token" },
            tokenRefresher = { TokenPair("fresh", "refresh", Long.MAX_VALUE) },
            onNetworkFailure = { failures.incrementAndGet() },
        )

        assertFailsWith<AppError.Network> {
            client.executeWithRetry(HttpMethod.Get, "https://example.test/files", HttpSemantics.READ)
        }
        assertEquals(1, failures.get())
    }
}
