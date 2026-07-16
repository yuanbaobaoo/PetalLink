package io.github.yuanbaobaao.petallink.error

import io.github.yuanbaobaao.petallink.AppError
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlin.time.Duration.Companion.seconds

/**
 * ErrorSerializer 单测。
 */
class ErrorSerializerTest {

    @Test
    fun Network错误序列化() {
        val map = ErrorSerializer.toMap(AppError.Network("timeout"))
        assertEquals("NETWORK", map["kind"])
        assertEquals("timeout", map["message"])
        assertNull(map["status"])  // 非 Remote 无 status
    }

    @Test
    fun Remote错误带状态码() {
        val map = ErrorSerializer.toMap(AppError.Remote(500, "server error"))
        assertEquals("REMOTE", map["kind"])
        assertEquals(500, map["status"])
    }

    @Test
    fun Auth错误序列化() {
        val map = ErrorSerializer.toMap(AppError.Auth("token expired"))
        assertEquals("AUTH", map["kind"])
        assertEquals("token expired", map["message"])
    }

    @Test
    fun 带元数据时输出retryAfterMs() {
        val meta = ErrorMetadata(
            retryAfter = 30.seconds,
            transportKind = DriveTransportKind.TIMEOUT,
        )
        val map = ErrorSerializer.toMap(AppError.Remote(503, "unavailable"), meta)
        assertEquals(30000L, map["retryAfterMs"])
        assertEquals("TIMEOUT", map["transportKind"])
    }

    @Test
    fun 无元数据时不包含retryAfterMs() {
        val map = ErrorSerializer.toMap(AppError.Network("err"))
        assertTrue(!map.containsKey("retryAfterMs"))
    }
}
