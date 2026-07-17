package io.github.yuanbaobaao.petallink.auth

import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.engine.mock.toByteArray
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class UserInfoApiTest {
    @Test
    fun 三端点并发且OIDC失败不阻断其他资料() { runBlocking {
        val entered = AtomicInteger()
        val allEntered = CompletableDeferred<Unit>()
        val requestBodies = mutableListOf<String>()
        val engine = MockEngine { request ->
            if (entered.incrementAndGet() == 3) allEntered.complete(Unit)
            withTimeout(1_000) { allEntered.await() }

            val service = request.url.parameters["nsp_svc"]
            when (service) {
                "GOpen.User.getInfo" -> {
                    requestBodies += request.body.toByteArray().decodeToString()
                    respond(
                        """{"displayName":"匿名用户","displayNameFlag":1,"openID":"open-1","headPictureURL":"https://avatar"}""",
                        HttpStatusCode.OK,
                        headersOf(HttpHeaders.ContentType, "application/json"),
                    )
                }
                "GOpen.User.getPhone" -> {
                    requestBodies += request.body.toByteArray().decodeToString()
                    respond("13800138000", HttpStatusCode.OK)
                }
                else -> {
                    assertEquals("Bearer tok+en", request.headers[HttpHeaders.Authorization])
                    respond("not found", HttpStatusCode.NotFound)
                }
            }
        }
        val info = UserInfoApi(
            HttpClient(engine),
            tokenProvider = { "tok+en" },
            restPhpUrl = "https://account.test/rest.php",
            oidcUrl = "https://oauth.test/userinfo",
        ).get()

        assertEquals(3, entered.get())
        assertEquals("open-1", info.openId)
        assertEquals("13800138000", info.mobile)
        assertEquals("https://avatar", info.avatarUrl)
        assertTrue(info.isAnonymized)
        assertNull(info.displayName)
        assertTrue(requestBodies.all { it.contains("access_token=tok%2Ben") })
    } }

    @Test
    fun 合并优先级为oidc小于info小于phone() { runBlocking {
        val engine = MockEngine { request ->
            when (request.url.parameters["nsp_svc"]) {
                "GOpen.User.getInfo" -> respond(
                    """{"displayName":"Info","mobile":"info-phone","email":"info@example.test"}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
                "GOpen.User.getPhone" -> respond(
                    """{"mobile":"phone-wins"}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
                else -> respond(
                    """{"sub":"sub-1","displayName":"OIDC","email":"oidc@example.test"}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            }
        }
        val info = UserInfoApi(
            HttpClient(engine), { "token" },
            "https://account.test/rest.php", "https://oauth.test/userinfo",
        ).get()
        assertEquals("sub-1", info.sub)
        assertEquals("Info", info.displayName)
        assertEquals("info@example.test", info.email)
        assertEquals("phone-wins", info.mobile)
    } }
}
