package io.github.yuanbaobaao.petallink.drive

import io.github.yuanbaobaao.petallink.error.DriveTransportKind
import io.github.yuanbaobaao.petallink.error.RequestSemantics
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/**
 * ErrorClassifier + RetryAfter 单测（对标 docs/03 §HTTP 客户端）。
 */
class ErrorClassifierTest {

    @Test
    fun classify_连接失败优先级最高() {
        assertEquals(
            DriveTransportKind.CONNECTION,
            ErrorClassifier.classifyTransport(
                isConnect = true, isTimeout = true, isBody = true, isDecode = true, isRequest = true,
            ),
        )
    }

    @Test
    fun classify_超时优先于body() {
        assertEquals(
            DriveTransportKind.TIMEOUT,
            ErrorClassifier.classifyTransport(
                isConnect = false, isTimeout = true, isBody = true, isDecode = true, isRequest = true,
            ),
        )
    }

    @Test
    fun classify_body优先于decode() {
        assertEquals(
            DriveTransportKind.RESPONSE_BODY_NOT_IN_BRIEF,
            ErrorClassifier.classifyTransport(
                isConnect = false, isTimeout = false, isBody = true, isDecode = true, isRequest = true,
            ),
        )
    }

    @Test
    fun classify_decode优先于request() {
        assertEquals(
            DriveTransportKind.DECODE,
            ErrorClassifier.classifyTransport(
                isConnect = false, isTimeout = false, isBody = false, isDecode = true, isRequest = true,
            ),
        )
    }

    @Test
    fun classify_无标志返回Other() {
        assertEquals(
            DriveTransportKind.OTHER,
            ErrorClassifier.classifyTransport(false, false, false, false, false),
        )
    }

    @Test
    fun mayHaveReachedServer_写操作非连接失败为true() {
        assertEquals(
            true,
            ErrorClassifier.mayHaveReachedServer(RequestSemantics.WRITE_LIKE, DriveTransportKind.TIMEOUT),
        )
    }

    @Test
    fun mayHaveReachedServer_写操作连接失败为false() {
        assertEquals(
            false,
            ErrorClassifier.mayHaveReachedServer(RequestSemantics.WRITE_LIKE, DriveTransportKind.CONNECTION),
        )
    }

    @Test
    fun mayHaveReachedServer_读操作永远false() {
        assertEquals(
            false,
            ErrorClassifier.mayHaveReachedServer(RequestSemantics.READ_LIKE, DriveTransportKind.TIMEOUT),
        )
    }

    @Test
    fun parseRetryAfter_纯数字返回DelaySeconds() {
        val ra = ErrorClassifier.parseRetryAfter("30")
        assertEquals(RetryAfter.DelaySeconds(30L), ra)
    }

    @Test
    fun parseRetryAfter_空返回null() {
        assertNull(ErrorClassifier.parseRetryAfter(null))
        assertNull(ErrorClassifier.parseRetryAfter(""))
        assertNull(ErrorClassifier.parseRetryAfter("   "))
    }

    @Test
    fun parseRetryAfter_非数字非日期返回null() {
        assertNull(ErrorClassifier.parseRetryAfter("abc"))
    }

    @Test
    fun RetryAfter_DelaySeconds计算下次重试时间() {
        val ra = RetryAfter.DelaySeconds(10L)
        // 10 秒 = 10000 毫秒
        assertEquals(10000L, ra.nextRetryAt(0L))
    }

    @Test
    fun RetryAfter_AtUnixMs不早于now() {
        // timestampMs < nowMs → 返回 nowMs
        val ra = RetryAfter.AtUnixMs(timestampMs = 100L)
        assertEquals(200L, ra.nextRetryAt(200L))
    }
}
