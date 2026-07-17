package io.github.yuanbaobaao.petallink.core.logging

import java.io.PrintWriter
import java.io.StringWriter
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption
import java.time.Clock
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter

/** Logger 实例是轻量门面；所有实例共享一个应用级后端。 */
actual class Logger actual constructor() {
    actual fun trace(target: String, message: () -> String) = LoggerRuntime.log(LogLevel.TRACE, target, message, null)
    actual fun debug(target: String, message: () -> String) = LoggerRuntime.log(LogLevel.DEBUG, target, message, null)
    actual fun info(target: String, message: () -> String) = LoggerRuntime.log(LogLevel.INFO, target, message, null)
    actual fun warn(target: String, message: () -> String) = LoggerRuntime.log(LogLevel.WARN, target, message, null)
    actual fun error(target: String, message: () -> String, throwable: Throwable?) =
        LoggerRuntime.log(LogLevel.ERROR, target, message, throwable)

    fun snapshot(count: Int = 1000): List<LogRecord> = LoggerRuntime.snapshot(count)
    fun clear() = LoggerRuntime.clear()
}

/** console + daily file + 1000 条 ring buffer 的唯一运行时。 */
object LoggerRuntime {
    private const val MAX_LOG_DAYS = 30L
    private val lock = Any()
    private val ringBuffer = RingBufferAppender(1000)
    private var minLevel: LogLevel = LogLevel.INFO
    private var logDirectory: Path? = null
    private var clock: Clock = Clock.systemUTC()

    fun configure(directory: Path, level: LogLevel = LogLevel.INFO) = synchronized(lock) {
        Files.createDirectories(directory)
        logDirectory = directory
        minLevel = level
        clock = Clock.systemUTC()
        cleanupOldLogs(directory)
    }

    internal fun configureForTest(directory: Path, clock: Clock, level: LogLevel = LogLevel.INFO) =
        synchronized(lock) {
            Files.createDirectories(directory)
            logDirectory = directory
            minLevel = level
            this.clock = clock
            ringBuffer.clear()
            cleanupOldLogs(directory)
        }

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

    fun snapshot(count: Int = 1000): List<LogRecord> = ringBuffer.snapshot(count.coerceIn(0, 1000))

    fun clear() = synchronized(lock) {
        ringBuffer.clear()
        logDirectory?.let { directory ->
            if (Files.exists(directory)) Files.list(directory).use { files ->
                files.filter { it.fileName.toString().startsWith("PetalLink.log.") }
                    .forEach(Files::deleteIfExists)
            }
        }
    }

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

    private fun dailyFile(directory: Path, instant: Instant): Path {
        val date = DateTimeFormatter.ISO_LOCAL_DATE.withZone(ZoneOffset.UTC).format(instant)
        return directory.resolve("PetalLink.log.$date")
    }

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
