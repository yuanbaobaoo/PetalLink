package io.github.yuanbaobaoo.petallink.core.logging

import java.io.PrintWriter
import java.io.StringWriter
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.time.Clock
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

/**
 * Logger 实例是轻量门面；所有实例共享一个应用级后端。
 */
actual class Logger actual constructor() {
    /**
     * 记录 TRACE 级别日志。
     */
    actual fun trace(target: String, message: () -> String) = LoggerRuntime.log(LogLevel.TRACE, target, message, null)
    /**
     * 记录 DEBUG 级别日志。
     */
    actual fun debug(target: String, message: () -> String) = LoggerRuntime.log(LogLevel.DEBUG, target, message, null)
    /**
     * 记录 INFO 级别日志。
     */
    actual fun info(target: String, message: () -> String) = LoggerRuntime.log(LogLevel.INFO, target, message, null)
    /**
     * 记录 WARN 级别日志。
     */
    actual fun warn(target: String, message: () -> String) = LoggerRuntime.log(LogLevel.WARN, target, message, null)
    /**
     * 记录 ERROR 级别日志，可附带异常。
     */
    actual fun error(target: String, message: () -> String, throwable: Throwable?) =
        LoggerRuntime.log(LogLevel.ERROR, target, message, throwable)

    /**
     * 返回环形缓冲中最近 [count] 条日志记录。
     */
    fun snapshot(count: Int = 1000): List<LogRecord> = LoggerRuntime.snapshot(count)
    /**
     * 清空环形缓冲及磁盘日志文件。
     */
    fun clear() = LoggerRuntime.clear()
}

/**
 * console + daily file + 1000 条 ring buffer 的唯一运行时。
 */
object LoggerRuntime {
    private const val MAX_LOG_DAYS = 30L
    private val lock = Any()
    private val ringBuffer = RingBufferAppender(1000)
    private var minLevel: LogLevel = LogLevel.INFO
    private var logDirectory: Path? = null
    private var clock: Clock = Clock.systemUTC()

    /**
     * 配置日志目录与最低级别，并清理过期日志。
     */
    fun configure(directory: Path, level: LogLevel = LogLevel.INFO) = synchronized(lock) {
        Files.createDirectories(directory)
        logDirectory = directory
        minLevel = level
        clock = Clock.systemUTC()
        cleanupOldLogs(directory)
    }

    /**
     * 测试专用配置：注入可控时钟并重置环形缓冲。
     */
    internal fun configureForTest(directory: Path, clock: Clock, level: LogLevel = LogLevel.INFO) =
        synchronized(lock) {
            Files.createDirectories(directory)
            logDirectory = directory
            minLevel = level
            this.clock = clock
            ringBuffer.clear()
            cleanupOldLogs(directory)
        }

    /**
     * 核心日志写入：低于最低级别丢弃，经脱敏后同时输出到控制台、环形缓冲与按日文件
     */
    internal fun log(level: LogLevel, target: String, message: () -> String, throwable: Throwable?) {
        if (level.severity < minLevel.severity) return
        val safeTarget = LogRedactor.redact(target)
        val safeMessage = LogRedactor.redact(message())
        val timestamp = clock.millis()
        val record = LogRecord(timestamp, level, safeTarget, safeMessage, throwable)
        val instant = Instant.ofEpochMilli(timestamp)
        val prefix = "[${DateTimeFormatter.ISO_INSTANT.format(instant)}] [${level.name}] $safeTarget: $safeMessage"
        val stack = throwable?.let {
            val writer = StringWriter()
            it.printStackTrace(PrintWriter(writer))
            LogRedactor.redact(writer.toString())
        }
        val line = if (stack == null) prefix else "$prefix\n$stack"

        synchronized(lock) {
            println(line)
            ringBuffer.append(record)
            logDirectory?.let { directory ->
                Files.createDirectories(directory)
                Files.writeString(
                    dailyFile(directory, instant),
                    "$line\n",
                    StandardOpenOption.CREATE,
                    StandardOpenOption.APPEND,
                )
            }
        }
    }

    /**
     * 返回环形缓冲中最近 [count] 条记录（最多 1000 条）。
     */
    fun snapshot(count: Int = 1000): List<LogRecord> = ringBuffer.snapshot(count.coerceIn(0, 1000))

    /**
     * 清空环形缓冲并删除磁盘上的日志文件。
     */
    fun clear() = synchronized(lock) {
        ringBuffer.clear()
        logDirectory?.let { directory ->
            if (Files.exists(directory)) Files.list(directory).use { files ->
                files.filter { it.fileName.toString().startsWith("PetalLink.log.") }
                    .forEach(Files::deleteIfExists)
            }
        }
    }

    /**
     * 将所有按日日志按时间顺序合并导出到指定文件。
     */
    fun exportTo(destination: Path) = synchronized(lock) {
        destination.parent?.let(Files::createDirectories)
        val content = logDirectory?.let { directory ->
            if (!Files.exists(directory)) "" else Files.list(directory).use { files ->
                files.filter { it.fileName.toString().startsWith("PetalLink.log.") }
                    .sorted()
                    .map(Files::readString)
                    .toList()
                    .joinToString("")
            }
        } ?: ""
        Files.writeString(destination, content)
    }

    /**
     * 根据时间戳计算当天的日志文件路径（按 UTC 日期命名）。
     */
    private fun dailyFile(directory: Path, instant: Instant): Path {
        val date = DateTimeFormatter.ISO_LOCAL_DATE.withZone(ZoneOffset.UTC).format(instant)
        return directory.resolve("PetalLink.log.$date")
    }

    /**
     * 删除超过保留天数（[MAX_LOG_DAYS]）的按日日志文件。
     */
    private fun cleanupOldLogs(directory: Path) {
        val today = Instant.ofEpochMilli(clock.millis()).atZone(ZoneOffset.UTC).toLocalDate()
        Files.list(directory).use { files ->
            files.filter { it.fileName.toString().startsWith("PetalLink.log.") }.forEach { file ->
                val date = runCatching {
                    java.time.LocalDate.parse(file.fileName.toString().removePrefix("PetalLink.log."))
                }.getOrNull() ?: return@forEach
                if (java.time.temporal.ChronoUnit.DAYS.between(date, today) > MAX_LOG_DAYS) {
                    Files.deleteIfExists(file)
                }
            }
        }
    }
}
