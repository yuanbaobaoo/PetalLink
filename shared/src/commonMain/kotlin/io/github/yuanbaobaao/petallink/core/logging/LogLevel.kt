package io.github.yuanbaobaao.petallink.core.logging

/**
 * 日志级别（对标 src/core/logging.rs）。
 *
 * 默认级别 INFO；debug 模式也用 INFO（非 DEBUG）。
 */
enum class LogLevel(val severity: Int) {
    TRACE(0),
    DEBUG(1),
    INFO(2),
    WARN(3),
    ERROR(4);

    /** 当前级别是否 >= [level]（即会被输出） */
    fun isEnabled(level: LogLevel): Boolean = severity >= level.severity
}
