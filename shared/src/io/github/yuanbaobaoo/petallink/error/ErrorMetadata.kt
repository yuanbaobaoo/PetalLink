package io.github.yuanbaobaoo.petallink.error

import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds

/**
 * 错误恢复元数据（对标 src/error.rs 恢复信息）。
 *
 * 携带重试决策所需信息，供 retry_policy（阶段 4）使用。
 */
data class ErrorMetadata(
    /**
     * 服务端 Retry-After 头解析值（重试前等待时长）
     */
    val retryAfter: Duration? = null,

    /**
     * 请求语义：决定错误分类（可重试 / 需刷新 token / 不可恢复）
     */
    val requestSemantics: RequestSemantics = RequestSemantics.UNKNOWN,

    /**
     * 传输错误子类（DNS / 连接 / 超时 / 打断）
     */
    val transportKind: DriveTransportKind? = null,
)

/**
 * 请求语义（对标 src/error.rs RequestSemantics）
 */
enum class RequestSemantics {
    /**
     * 读类（GET/HEAD/OPTIONS）
     */
    READ_LIKE,

    /**
     * 写类（POST/PUT/PATCH/DELETE）
     */
    WRITE_LIKE,

    /**
     * 可重试（瞬时错误）
     */
    RETRYABLE,

    /**
     * 需刷新 token 后重试（401）
     */
    NEEDS_TOKEN_REFRESH,

    /**
     * 不可恢复（永久错误，如 4xx 非重试类）
     */
    NON_RECOVERABLE,

    /**
     * 未分类
     */
    UNKNOWN,
}

/**
 * 传输错误子类（对标 src/error.rs DriveTransportKind）
 */
enum class DriveTransportKind {
    /**
     * 域名解析失败
     */
    DNS,

    /**
     * 连接失败（拒绝/重置）
     */
    CONNECTION,

    /**
     * 超时
     */
    TIMEOUT,

    /**
     * 连接被打断
     */
    INTERRUPTED,

    /**
     * 响应体读取失败
     */
    RESPONSE_BODY_NOT_IN_BRIEF,

    /**
     * 解码失败
     */
    DECODE,

    /**
     * 请求构造失败
     */
    REQUEST,

    /**
     * 其他
     */
    OTHER,
}
