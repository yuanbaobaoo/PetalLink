package io.github.yuanbaobaao.petallink.platform

import java.nio.file.Files
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.io.path.createTempDirectory
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class DesktopLifecycleTest {
    @Test
    fun 第二实例通过loopback通知主实例且不能取得锁() {
        val dir = createTempDirectory("petallink-instance-")
        val shown = AtomicBoolean(false)
        val primary = SingleInstanceCoordinator(dir) { shown.set(true) }
        val secondary = SingleInstanceCoordinator(dir) {}
        try {
            assertTrue(primary.acquireOrNotify())
            assertFalse(secondary.acquireOrNotify())
            repeat(20) {
                if (shown.get()) return@repeat
                Thread.sleep(25)
            }
            assertTrue(shown.get())
            assertTrue(Files.isRegularFile(dir.resolve("instance.port")))
        } finally {
            secondary.close()
            primary.close()
            dir.toFile().deleteRecursively()
        }
    }

    @Test
    fun LaunchAgent原子写入hidden参数并可禁用() {
        val dir = createTempDirectory("petallink-launchagent-")
        val manager = LaunchAgentManager("test.petallink", dir.resolve("PetalLink"), dir)
        try {
            manager.setEnabled(true)
            assertTrue(manager.isEnabled())
            val content = Files.readString(manager.plistPath)
            assertTrue(content.contains("--hidden"))
            assertTrue(content.contains("test.petallink"))
            manager.setEnabled(false)
            assertFalse(manager.isEnabled())
        } finally {
            dir.toFile().deleteRecursively()
        }
    }
}
