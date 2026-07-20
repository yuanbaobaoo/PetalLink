package io.github.yuanbaobaoo.petallink.sync

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.config.AppConfig
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
 * - 写操作网络错误按「请求是否可能已送达」分流：可能送达 → VerifyingRemote，确定未送达 → WaitingForNetwork
 */
object RetryPolicy {

    /**
     * 计算第 [attempt] 次重试前的退避时长（attempt 从 0 开始计）。
     * 公式：base = 2^attempt 秒，封顶 300s，再加 0~1s 的 jitter。
     */
    fun backoff(attempt: Int, jitterMs: Long = Random.nextLong(0, 1000)): Duration {
        val exp = 1L shl min(attempt, 30)              // 2^attempt，防溢出封到 2^30
        val baseSeconds = min(exp, AppConfig.BACKOFF_CAP.inWholeSeconds)
        return baseSeconds.seconds + jitterMs.coerceIn(0L, 999L).milliseconds
    }

    /**
     * 传输失败后的恢复决策（对标 retry_policy.rs RecoveryDecision 的 CMP 子集）。
     */
    enum class RecoveryDecision {
        /**
         * 等待网络恢复（请求确定未送达，可安全盲重放）
         */
        WAIT_FOR_NETWORK,

        /**
         * 指数退避后重试
         */
        BACKOFF,

        /**
         * 请求可能已到达服务端，必须先核验远端写入结果，禁止盲重放
         */
        VERIFY_REMOTE,

        /**
         * 永久失败（终态）
         */
        FAIL,
    }

    // 对标 retry_policy.rs:167：仅这四种 5xx 在预算耗尽时允许转远端核验
    private val VERIFY_ON_EXHAUSTED_STATUSES = setOf(500, 502, 503, 504)

    /**
     * 传输错误分类（对标 retry_policy.rs classify_transport / classify_status 的写安全规则）。
     *
     * @param error 传输过程抛出的结构化错误
     * @param modifiesRemote 该传输是否可能改变云端状态（上传/删除为 true，下载为 false）
     * @param budgetExhausted 自动重试预算是否已耗尽（attempt + 1 >= MAX_AUTOMATIC_ATTEMPTS）
     */
    fun classifyTransferError(
        error: AppError,
        modifiesRemote: Boolean,
        budgetExhausted: Boolean,
    ): RecoveryDecision = when {
        // 写响应丢失或成功响应无法核验：只能核验远端（含 upload_session_expired，retry_policy.rs:62-68）
        error is AppError.RemoteAmbiguous -> RecoveryDecision.VERIFY_REMOTE
        error.kind == AppError.ErrorKind.NETWORK ->
            // retry_policy.rs:97-140：写操作 + 非连接建立期失败 → 可能已送达 → VerifyRemote
            if (modifiesRemote && requestMayHaveReachedServer(error)) RecoveryDecision.VERIFY_REMOTE
            else RecoveryDecision.WAIT_FOR_NETWORK
        error.kind == AppError.ErrorKind.AUTH -> RecoveryDecision.FAIL
        error.kind == AppError.ErrorKind.REMOTE -> {
            val status = (error as? AppError.Remote)?.status ?: 0
            when {
                status == 408 || status == 429 || status in 500..599 ->
                    if (!budgetExhausted) {
                        RecoveryDecision.BACKOFF
                    } else if (modifiesRemote && status in VERIFY_ON_EXHAUSTED_STATUSES) {
                        // retry_policy.rs:167-177：预算耗尽且写入可能已送达 → VerifyRemote
                        RecoveryDecision.VERIFY_REMOTE
                    } else {
                        RecoveryDecision.FAIL
                    }
                else -> RecoveryDecision.FAIL
            }
        }
        else -> RecoveryDecision.FAIL
    }

    /**
     * 判定网络错误是否处于「请求可能已到达服务端」阶段（对标 ErrorClassifier.mayHaveReachedServer）。
     *
     * 连接建立期失败（DNS 解析失败 / 拒绝连接 / 无路由 / 连接超时）→ 请求确定未送达；
     * 其余阶段（读写超时、连接中断、响应体读取失败等）→ 可能已送达。
     */
    fun requestMayHaveReachedServer(error: AppError): Boolean {
        var cause: Throwable? = error
        while (cause != null) {
            when (cause) {
                is java.net.UnknownHostException,
                is java.net.NoRouteToHostException,
                is java.net.ConnectException,
                is io.ktor.client.network.sockets.ConnectTimeoutException,
                -> return false
            }
            cause = cause.cause
        }
        return true
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
