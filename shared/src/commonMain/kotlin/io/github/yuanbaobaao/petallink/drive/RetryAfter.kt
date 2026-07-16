package io.github.yuanbaobaao.petallink.drive

import io.github.yuanbaobaao.petallink.error.DriveTransportKind
import io.github.yuanbaobaao.petallink.error.RequestSemantics

/**
 * Retry-After 头解析（对标原项目 error.rs parse_retry_after）。
 *
 * 详见 docs/03 §HTTP 客户端。
 */
sealed class RetryAfter {
    /** 延迟指定秒数 */
    data class DelaySeconds(val seconds: Long) : RetryAfter()
    /** 在指定 Unix 毫秒时间戳重试 */
    data class AtUnixMs(val timestampMs: Long) : RetryAfter()

    /** 计算下次重试的毫秒时间戳 */
    fun nextRetryAt(nowMs: Long): Long = when (this) {
        is DelaySeconds -> nowMs + seconds * 1000
        is AtUnixMs -> maxOf(timestampMs, nowMs)
    }
}

/**
 * HTTP 错误分类纯逻辑（对标原项目 drive/client.rs classify_transport_error）。
 *
 * 优先级（高→低）：Connect > Timeout > ResponseBody > Decode > Request > Other
 */
object ErrorClassifier {

    /**
     * 分类传输错误。
     * @param isConnect 是否连接失败（拒绝/重置）
     * @param isTimeout 是否超时
     * @param isBody 是否响应体读取失败
     * @param isDecode 是否解码失败
     * @param isRequest 是否请求构造失败
     */
    fun classifyTransport(
        isConnect: Boolean,
        isTimeout: Boolean,
        isBody: Boolean,
        isDecode: Boolean,
        isRequest: Boolean,
    ): DriveTransportKind = when {
        isConnect -> DriveTransportKind.CONNECTION
        isTimeout -> DriveTransportKind.TIMEOUT
        isBody -> DriveTransportKind.RESPONSE_BODY_NOT_IN_BRIEF
        isDecode -> DriveTransportKind.DECODE
        isRequest -> DriveTransportKind.REQUEST
        else -> DriveTransportKind.OTHER
    }

    /**
     * 判定"请求可能已到达服务端"（docs/03 §写操作核验）。
     * Write + 非 Connect 失败 → 可能已提交 → 恢复层必须 VerifyRemote。
     */
    fun mayHaveReachedServer(semantics: RequestSemantics, transportKind: DriveTransportKind): Boolean =
        semantics == RequestSemantics.WRITE_LIKE && transportKind != DriveTransportKind.CONNECTION

    /**
     * 解析 Retry-After 头值。
     * - 纯数字 → DelaySeconds
     * - RFC2822 日期 → AtUnixMs
     * - 其他 → null
     */
    fun parseRetryAfter(value: String?): RetryAfter? {
        if (value.isNullOrBlank()) return null
        val trimmed = value.trim()
        // 纯数字 → 秒
        trimmed.toLongOrNull()?.let { return RetryAfter.DelaySeconds(it) }
        // TODO(stage2): RFC2822 日期解析 → AtUnixMs（需 kotlinx-datetime 或手写）
        // 当前仅支持秒数形式
        return null
    }
}
