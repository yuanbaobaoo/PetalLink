package io.github.yuanbaobaoo.petallink.sync

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.sync.RetryPolicy.RecoveryDecision
import java.net.ConnectException
import java.net.NoRouteToHostException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * RetryPolicy 写安全错误分类单测（§3.3，对标 retry_policy.rs:62-68,97-140,167-177）。
 */
class RetryPolicyTest {
    @Test
    fun 连接建立期失败判定为未送达() {
        assertFalse(RetryPolicy.requestMayHaveReachedServer(AppError.Network("拒绝连接", ConnectException("refused"))))
        assertFalse(RetryPolicy.requestMayHaveReachedServer(AppError.Network("DNS 失败", UnknownHostException("drive"))))
        assertFalse(RetryPolicy.requestMayHaveReachedServer(AppError.Network("无路由", NoRouteToHostException())))
    }

    @Test
    fun 非连接期网络失败判定为可能已送达() {
        assertTrue(RetryPolicy.requestMayHaveReachedServer(AppError.Network("读写超时", SocketTimeoutException("timeout"))))
        assertTrue(RetryPolicy.requestMayHaveReachedServer(AppError.Network("连接中断", java.net.SocketException("Connection reset"))))
        assertTrue(RetryPolicy.requestMayHaveReachedServer(AppError.Network("未知网络错误")))
    }

    @Test
    fun 写操作网络错误按送达可能性分流() {
        val reached = AppError.Network("读写超时", SocketTimeoutException("timeout"))
        assertEquals(
            RecoveryDecision.VERIFY_REMOTE,
            RetryPolicy.classifyTransferError(reached, modifiesRemote = true, budgetExhausted = false),
        )
        assertEquals(
            RecoveryDecision.WAIT_FOR_NETWORK,
            RetryPolicy.classifyTransferError(reached, modifiesRemote = false, budgetExhausted = false),
        )
        val notReached = AppError.Network("拒绝连接", ConnectException("refused"))
        assertEquals(
            RecoveryDecision.WAIT_FOR_NETWORK,
            RetryPolicy.classifyTransferError(notReached, modifiesRemote = true, budgetExhausted = false),
        )
    }

    @Test
    fun 服务端5xx预算耗尽且写入可能已送达转远端核验() {
        for (status in listOf(500, 502, 503, 504)) {
            assertEquals(
                RecoveryDecision.VERIFY_REMOTE,
                RetryPolicy.classifyTransferError(AppError.Remote(status, "服务端错误"), true, budgetExhausted = true),
                "status=$status",
            )
            assertEquals(
                RecoveryDecision.BACKOFF,
                RetryPolicy.classifyTransferError(AppError.Remote(status, "服务端错误"), true, budgetExhausted = false),
                "status=$status",
            )
        }
        // 读操作预算耗尽 → 终态失败；非白名单 5xx（如 501）耗尽也失败
        assertEquals(
            RecoveryDecision.FAIL,
            RetryPolicy.classifyTransferError(AppError.Remote(503, "服务端错误"), false, budgetExhausted = true),
        )
        assertEquals(
            RecoveryDecision.FAIL,
            RetryPolicy.classifyTransferError(AppError.Remote(501, "服务端错误"), true, budgetExhausted = true),
        )
    }

    @Test
    fun 四二九预算内退避预算耗尽失败() {
        assertEquals(
            RecoveryDecision.BACKOFF,
            RetryPolicy.classifyTransferError(AppError.Remote(429, "限流"), true, budgetExhausted = false),
        )
        assertEquals(
            RecoveryDecision.FAIL,
            RetryPolicy.classifyTransferError(AppError.Remote(429, "限流"), true, budgetExhausted = true),
        )
    }

    @Test
    fun 上传会话失效只能远端核验() {
        // upload_session_expired 在 CMP 以 RemoteAmbiguous 承载（retry_policy.rs:62-68 → VerifyRemote）
        val expired = AppError.RemoteAmbiguous("上传会话已失效 (404)，丢弃持久化会话身份前必须复核远端写入")
        assertEquals(
            RecoveryDecision.VERIFY_REMOTE,
            RetryPolicy.classifyTransferError(expired, modifiesRemote = true, budgetExhausted = false),
        )
        assertEquals(
            RecoveryDecision.VERIFY_REMOTE,
            RetryPolicy.classifyTransferError(expired, modifiesRemote = true, budgetExhausted = true),
        )
    }

    @Test
    fun 鉴权与四xx业务错误不可重试() {
        assertEquals(
            RecoveryDecision.FAIL,
            RetryPolicy.classifyTransferError(AppError.Auth("token 失效"), true, budgetExhausted = false),
        )
        assertEquals(
            RecoveryDecision.FAIL,
            RetryPolicy.classifyTransferError(AppError.Remote(400, "参数错误"), true, budgetExhausted = false),
        )
        assertEquals(
            RecoveryDecision.FAIL,
            RetryPolicy.classifyTransferError(AppError.Data("schema 错误"), true, budgetExhausted = false),
        )
    }
}
