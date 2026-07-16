package io.github.yuanbaobaao.petallink.sync

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.config.AppConfig
import kotlin.math.min
import kotlin.random.Random
import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.Duration.Companion.seconds

/**
 * 传输重试与退避策略（对标原项目 src/sync/retry_policy.rs）
 *
 * 详见 docs/06 §9。核心：
 * - 错误分类决定可重试性（classifyTransport / classifyStatus）
 * - 退避 = 2^attempt 秒，上限 300s，加 jitter
 * - attempt 超过 MAX_AUTOMATIC_ATTEMPTS(5) → Failed
 */
object RetryPolicy {

    /**
     * 计算第 [attempt] 次重试前的退避时长（attempt 从 0 开始计）。
     * 公式：base = 2^attempt 秒，封顶 300s，再加 0~1s 的 jitter。
     */
    fun backoff(attempt: Int): Duration {
        val exp = 1L shl min(attempt, 30)              // 2^attempt，防溢出封到 2^30
        val baseSeconds = min(exp, AppConfig.BACKOFF_CAP.inWholeSeconds)
        val jitterMs = Random.nextLong(0, 1000)        // 0~999ms 抖动
        return baseSeconds.seconds + jitterMs.milliseconds
    }

    /**
     * 判定错误是否可自动重试。
     * - NETWORK / Remote(5xx / 408 / 429) → 可重试
     * - AUTH → 不重试（交给 token 刷新流程后由上层重新入队）
     * - DATA / CONFLICT / Canceled → 不可重试
     */
    fun isRetryable(error: AppError): Boolean = when (error.kind) {
        AppError.ErrorKind.NETWORK -> true
        AppError.ErrorKind.REMOTE -> {
            val s = (error as? AppError.Remote)?.status ?: 0
            s == 408 || s == 429 || s in 500..599
        }
        AppError.ErrorKind.AUTH,
        AppError.ErrorKind.CONFLICT,
        AppError.ErrorKind.DATA,
        AppError.ErrorKind.CANCELED,
        AppError.ErrorKind.INTERNAL,
        AppError.ErrorKind.LOCAL_IO -> false
    }

    /**
     * 判定在第 [attempt] 次失败后是否仍可继续自动重试。
     * 超过 MAX_AUTOMATIC_ATTEMPTS 则进入 Failed 终态。
     */
    fun shouldContinueRetrying(attempt: Int, error: AppError): Boolean {
        if (!isRetryable(error)) return false
        return attempt < AppConfig.MAX_AUTOMATIC_ATTEMPTS
    }
}
