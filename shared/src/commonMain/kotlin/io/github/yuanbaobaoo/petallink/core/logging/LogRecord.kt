package io.github.yuanbaobaoo.petallink.core.logging

/**
 * 一条日志记录（对标 src/core/logging.rs LogRecord）。
 */
data class LogRecord(
    val timestampMs: Long,      // 毫秒时间戳
    val level: LogLevel,
    val target: String,         // 模块/标签（如 "drive.client"）
    val message: String,        // 日志正文（敏感字段须已脱敏）
    val throwable: Throwable? = null,
)
