package io.github.yuanbaobaoo.petallink.core.logging

import java.nio.file.Files
import java.time.Clock
import java.time.Instant
import java.time.ZoneOffset
import kotlin.io.path.readText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class LoggerTest {
    @Test
    fun 多个Logger共享缓冲并同时写每日文件且自动脱敏() {
        val dir = Files.createTempDirectory("petallink-log-test-")
        val instant = Instant.parse("2026-07-16T09:30:00Z")
        LoggerRuntime.configureForTest(dir, Clock.fixed(instant, ZoneOffset.UTC))

        Logger().info("auth") { "access_token=very-secret" }
        Logger().error("drive", { "authorization: Bearer-abc" })

        val snapshot = Logger().snapshot()
        assertEquals(2, snapshot.size)
        assertEquals("drive", snapshot.first().target)
        assertFalse(snapshot.any { it.message.contains("very-secret") || it.message.contains("Bearer-abc") })
        val file = dir.resolve("PetalLink.log.2026-07-16")
        assertTrue(Files.exists(file))
        assertFalse(file.readText().contains("very-secret"))
    }

    @Test
    fun 默认Info会保留Error但过滤Debug() {
        val dir = Files.createTempDirectory("petallink-log-test-")
        LoggerRuntime.configureForTest(dir, Clock.systemUTC(), LogLevel.INFO)
        Logger().debug("test") { "debug" }
        Logger().error("test", { "error" })
        assertEquals(listOf(LogLevel.ERROR), Logger().snapshot().map { it.level })
    }

    @Test
    fun 跨日期写不同文件并清理30天以前日志() {
        val dir = Files.createTempDirectory("petallink-log-test-")
        val old = dir.resolve("PetalLink.log.2026-05-01")
        Files.writeString(old, "old")
        LoggerRuntime.configureForTest(
            dir,
            Clock.fixed(Instant.parse("2026-07-16T23:59:00Z"), ZoneOffset.UTC),
        )
        assertFalse(Files.exists(old))
        Logger().info("test") { "day-one" }
        LoggerRuntime.configureForTest(
            dir,
            Clock.fixed(Instant.parse("2026-07-17T00:01:00Z"), ZoneOffset.UTC),
        )
        Logger().info("test") { "day-two" }
        assertTrue(Files.exists(dir.resolve("PetalLink.log.2026-07-16")))
        assertTrue(Files.exists(dir.resolve("PetalLink.log.2026-07-17")))
    }
}
