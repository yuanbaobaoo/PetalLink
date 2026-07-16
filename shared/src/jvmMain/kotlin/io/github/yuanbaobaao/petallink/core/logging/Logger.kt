package io.github.yuanbaobaao.petallink.core.logging

import java.io.PrintWriter
import java.io.StringWriter
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/**
 * JVM 日志实现（actual）。
 * 路由到 console + ringBuffer。token 脱敏。
 */
actual class Logger actual constructor() {
    private val minLevel: LogLevel = LogLevel.INFO
    private val ringBuffer = RingBufferAppender()

    actual fun trace(target: String, message: () -> String) = log(LogLevel.TRACE, target, message(), null)
    actual fun debug(target: String, message: () -> String) = log(LogLevel.DEBUG, target, message(), null)
    actual fun info(target: String, message: () -> String) = log(LogLevel.INFO, target, message(), null)
    actual fun warn(target: String, message: () -> String) = log(LogLevel.WARN, target, message(), null)
    actual fun error(target: String, message: () -> String, throwable: Throwable?) = log(LogLevel.ERROR, target, message(), throwable)

    private fun log(level: LogLevel, target: String, rawMessage: String, throwable: Throwable?) {
        if (!minLevel.isEnabled(level)) return
        val safeMessage = LogRedactor.redact(rawMessage)
        val ts = DateTimeFormatter.ISO_INSTANT.format(Instant.now())
        val record = LogRecord(System.currentTimeMillis(), level, target, safeMessage, throwable)
        println("[$ts] [${level.name}] $target: ${record.message}")
        throwable?.printStackTrace()
        ringBuffer.append(record)
    }

    fun snapshot(count: Int = 1000): List<LogRecord> = ringBuffer.snapshot(count)
}
