package io.github.yuanbaobaoo.petallink.auth

import io.github.yuanbaobaoo.petallink.AppError
import java.net.ServerSocket
import java.net.Socket
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class OauthServerTest {
    @Test
    fun state不匹配必须拒绝授权码() {
        assertFailsWith<AppError.Auth> {
            OauthCallbackValidator.requireCode(
                OauthServer.CallbackResult("code", "attacker", null, null),
                "expected",
            )
        }
    }

    @Test
    fun loopback回调解析code和state并保留加号() { runBlocking {
        val port = freePort()
        val server = OauthServer(port, 2_000)
        server.bind()
        val waiting = async(Dispatchers.IO) { server.waitForCallback() }
        Socket("127.0.0.1", port).use { socket ->
            socket.getOutputStream().write(
                "GET /oauth/callback?code=a%2Bb&state=s HTTP/1.1\r\nHost: localhost\r\n\r\n".toByteArray(),
            )
            socket.getOutputStream().flush()
            socket.getInputStream().readBytes()
        }
        val result = waiting.await()
        assertEquals("a+b", result.code)
        assertEquals("s", result.state)
    } }

    @Test
    fun 超时会关闭listener() { runBlocking {
        val server = OauthServer(freePort(), 30)
        assertFailsWith<AppError.Auth> { server.waitForCallback() }
        server.stop()
    } }

    @Test
    fun 主动取消会立即关闭listener并结束等待() { runBlocking {
        val server = OauthServer(freePort(), 5_000)
        server.bind()
        val waiting = async(Dispatchers.IO) {
            assertFailsWith<AppError.Canceled> { server.waitForCallback() }
        }
        delay(50)
        server.stop()
        waiting.await()
    } }

    private fun freePort(): Int = ServerSocket(0).use { it.localPort }
}
