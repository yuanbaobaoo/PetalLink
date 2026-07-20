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
import kotlin.test.assertFalse
import kotlin.test.assertTrue

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

    @Test
    fun 纯元数据事件被忽略不触发重扫() = runBlocking {
        val root = Files.createTempDirectory("petallink-watcher-kind-")
        val factory = FakeFactory()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val watcher = JvmLocalWatcher(root, scope, sourceFactory = factory, debounceMs = 20, warmupMs = 20)
        watcher.start()
        delay(40)
        val result = async { withTimeout(2_000) { watcher.changes.first { it.isNotEmpty() } } }
        // 占位符创建 / markDownloaded 写 xattr 只产生元数据位事件，必须被忽略
        factory.latest.emit(root.resolve("meta.txt").toString(), NativeFSEvent.FLAG_ITEM_XATTR_MOD or NativeFSEvent.FLAG_ITEM_IS_FILE)
        factory.latest.emit(root.resolve("chown.txt").toString(), NativeFSEvent.FLAG_ITEM_CHANGE_OWNER or NativeFSEvent.FLAG_ITEM_IS_FILE)
        delay(60)
        factory.latest.emit(root.resolve("real.txt").toString())
        assertEquals(listOf("real.txt"), result.await())
        watcher.close()
        scope.cancel()
    }

    @Test
    fun 事件kind映射只放行创建修改删除改名与流级重扫() {
        val file = NativeFSEvent.FLAG_ITEM_IS_FILE
        assertTrue(NativeFSEvent("a", NativeFSEvent.FLAG_ITEM_CREATED or file, 1UL).isChangeEvent())
        assertTrue(NativeFSEvent("a", NativeFSEvent.FLAG_ITEM_REMOVED or file, 1UL).isChangeEvent())
        assertTrue(NativeFSEvent("a", NativeFSEvent.FLAG_ITEM_RENAMED or file, 1UL).isChangeEvent())
        assertTrue(NativeFSEvent("a", NativeFSEvent.FLAG_ITEM_MODIFIED or file, 1UL).isChangeEvent())
        assertTrue(NativeFSEvent("a", NativeFSEvent.FLAG_MUST_SCAN_SUB_DIRS, 1UL).isChangeEvent())
        assertTrue(NativeFSEvent("a", NativeFSEvent.FLAG_ROOT_CHANGED, 1UL).isChangeEvent())
        assertFalse(NativeFSEvent("a", NativeFSEvent.FLAG_ITEM_XATTR_MOD or file, 1UL).isChangeEvent())
        assertFalse(NativeFSEvent("a", NativeFSEvent.FLAG_ITEM_INODE_META_MOD or file, 1UL).isChangeEvent())
        assertFalse(NativeFSEvent("a", NativeFSEvent.FLAG_ITEM_FINDER_INFO_MOD or file, 1UL).isChangeEvent())
        assertFalse(NativeFSEvent("a", NativeFSEvent.FLAG_ITEM_CHANGE_OWNER or file, 1UL).isChangeEvent())
        assertFalse(NativeFSEvent("a", file, 1UL).isChangeEvent())
        assertFalse(NativeFSEvent("a", 0, 1UL).isChangeEvent())
    }

    private class FakeFactory : FSEventSourceFactory {
        val sources = mutableListOf<FakeSource>()
        val latest: FakeSource get() = sources.last()
        override fun start(paths: List<String>, callback: (NativeFSEvent) -> Unit): AutoCloseable =
            FakeSource(callback).also(sources::add)
    }

    private class FakeSource(private val callback: (NativeFSEvent) -> Unit) : AutoCloseable {
        private var closed = false
        fun emit(path: String, flags: Int = NativeFSEvent.FLAG_ITEM_MODIFIED or NativeFSEvent.FLAG_ITEM_IS_FILE) {
            // 故意允许 close 后发回调，验证 generation 是最后一道防线。
            callback(NativeFSEvent(path, flags, 1UL))
        }
        override fun close() { closed = true }
    }
}
