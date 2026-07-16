package io.github.yuanbaobaao.petallink.core.logging

/**
 * 日志门面（expect，macosMain 提供 actual 路由到各 appender）。
 *
 * 用法：`Logger.info("drive.client") { "request sent" }`
 * message 用 lambda 延迟构造，避免低级别日志的字符串拼接开销。
 *
 * ⚠️ 安全约束（coding-rules）：所有 message 自动经 LogRedactor.redact 脱敏，
 * token/secret 绝不打印。但仍建议调用方不要把完整 token 拼进日志。
 */
expect class Logger() {
    fun trace(target: String, message: () -> String)
    fun debug(target: String, message: () -> String)
    fun info(target: String, message: () -> String)
    fun warn(target: String, message: () -> String)
    fun error(target: String, message: () -> String, throwable: Throwable? = null)
}

/**
 * 环形缓冲 appender（commonMain 默认实现，供日志查看页读取）。
 *
 * 容量 1000，newest-first。用 AtomicReference + 不可变 List 实现无锁线程安全
 * （KMP commonMain 无 synchronized，故用 CAS 循环）。
 */
class RingBufferAppender(private val capacity: Int = 1000) : LogAppender {
    // newest-first：index 0 是最新。整体是不可变 List，通过 CAS 替换。
    private val state = java.util.concurrent.atomic.AtomicReference<List<LogRecord>>(emptyList())

    override fun append(record: LogRecord) {
        while (true) {
            val current = state.get()
            val next = listOf(record) + current  // newest-first
            val trimmed = if (next.size > capacity) next.take(capacity) else next
            if (state.compareAndSet(current, trimmed)) break
        }
    }

    /** 取最近 [count] 条日志（newest-first） */
    fun snapshot(count: Int = capacity): List<LogRecord> = state.get().take(count)

    /** 清空缓冲 */
    fun clear() {
        state.set(emptyList())
    }
}
