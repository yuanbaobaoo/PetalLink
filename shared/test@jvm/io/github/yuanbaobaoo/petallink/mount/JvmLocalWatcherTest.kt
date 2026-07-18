package io.github.yuanbaobaoo.petallink.mount

import java.nio.file.Files
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals

class JvmLocalWatcherTest {
    @Test
    fun warmup丢弃历史事件但结束后事件按3s语义debounce() = runBlocking {
        val root = Files.createTempDirectory("petallink-watcher-unit-")
        val factory = FakeFactory()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val watcher = JvmLocalWatcher(root, scope, sourceFactory = factory, debounceMs = 30, warmupMs = 40)
        val result = async { withTimeout(2_000) { watcher.changes.first { it.isNotEmpty() } } }
        watcher.start()
        factory.latest.emit(root.resolve("warmup.txt").toString())
        delay(60)
        factory.latest.emit(root.resolve("b.txt").toString())
        factory.latest.emit(root.resolve("a.txt").toString())
        factory.latest.emit(root.resolve(".hwcloud_internal").toString())
        assertEquals(listOf("a.txt", "b.txt"), result.await())
        watcher.close()
        scope.cancel()
    }

    @Test
    fun restart后旧generation回调不得发布() = runBlocking {
        val root = Files.createTempDirectory("petallink-watcher-generation-")
        val factory = FakeFactory()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val watcher = JvmLocalWatcher(root, scope, sourceFactory = factory, debounceMs = 20, warmupMs = 20)
        watcher.start()
        val old = factory.latest
        delay(30)
        watcher.start()
        val current = factory.latest
        delay(30)
        val result = async { withTimeout(2_000) { watcher.changes.first { it.isNotEmpty() } } }
        old.emit(root.resolve("old.txt").toString())
        current.emit(root.resolve("new.txt").toString())
        assertEquals(listOf("new.txt"), result.await())
        watcher.close()
        scope.cancel()
    }

    private class FakeFactory : FSEventSourceFactory {
        val sources = mutableListOf<FakeSource>()
        val latest: FakeSource get() = sources.last()
        override fun start(paths: List<String>, callback: (NativeFSEvent) -> Unit): AutoCloseable =
            FakeSource(callback).also(sources::add)
    }

    private class FakeSource(private val callback: (NativeFSEvent) -> Unit) : AutoCloseable {
        private var closed = false
        fun emit(path: String) {
            // 故意允许 close 后发回调，验证 generation 是最后一道防线。
            callback(NativeFSEvent(path, 0, 1UL))
        }
        override fun close() { closed = true }
    }
}
