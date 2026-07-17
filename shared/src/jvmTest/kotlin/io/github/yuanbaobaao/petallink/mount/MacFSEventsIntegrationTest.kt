package io.github.yuanbaobaao.petallink.mount

import java.nio.file.Files
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.test.Test
import kotlin.test.assertTrue

class MacFSEventsIntegrationTest {
    @Test
    fun 真实FSEventStream可递归收到文件事件() {
        if (!System.getProperty("os.name").contains("Mac", ignoreCase = true)) return
        val root = Files.createTempDirectory("petallink-fsevents-native-")
        val realRoot = root.toRealPath()
        val events = LinkedBlockingQueue<NativeFSEvent>()
        MacFSEventSourceFactory.start(listOf(realRoot.toString()), events::offer).use {
            val nested = Files.createDirectories(realRoot.resolve("nested"))
            val target = Files.writeString(nested.resolve("event.txt"), "event")
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
            var matched = false
            while (!matched && System.nanoTime() < deadline) {
                val event = events.poll(250, TimeUnit.MILLISECONDS) ?: continue
                matched = event.path == target.toString() || event.path == nested.toString()
            }
            assertTrue(matched, "5 秒内未收到递归 FSEvents 回调")
        }
    }
}
