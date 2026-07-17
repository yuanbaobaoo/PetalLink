package io.github.yuanbaobaoo.petallink.core.logging

/**
 * 日志输出后端接口（commonMain 定义，macosMain 实现）。
 *
 * 三种实现（对标 src/core/logging.rs 三层）：
 * - ConsoleAppender：stdout 格式化输出
 * - FileAppender：滚动文件（PetalLink.log，按日轮转保留 30 天）
 * - RingBufferAppender：环形缓冲（MAX_BUFFER_SIZE=1000，newest-first，供日志查看页）
 */
interface LogAppender {
    /**
     * 输出一条日志
     */
    fun append(record: LogRecord)
}

/**
 * 日志脱敏钩子（coding-rules.md 硬约束：token/secret 绝不打印）。
 *
 * 所有 message 在进入 appender 前应先经 [redact] 处理。
 * 默认实现遮蔽常见敏感模式（token=xxx、Authorization: xxx）。
 */
object LogRedactor {
    /**
     * 敏感字段名（出现则遮蔽值）
     */
    private val SENSITIVE_KEYS = setOf(
        "token", "access_token", "refresh_token", "authorization",
        "password", "secret", "api_key", "apikey",
    )

    /**
     * 脱敏：把 message 中 `key=value` / `"key":"value"` 形式的敏感字段值替换为 ***。
     */
    fun redact(message: String): String {
        var result = message
        for (key in SENSITIVE_KEYS) {
            // 匹配 key=value 或 key":"value" 两种形式
            result = result.replace(
                Regex("($key)\\s*[:=]\\s*[\"']?[^\"'&\\s]+", RegexOption.IGNORE_CASE),
                "$1=***REDACTED***",
            )
        }
        return result
    }
}
