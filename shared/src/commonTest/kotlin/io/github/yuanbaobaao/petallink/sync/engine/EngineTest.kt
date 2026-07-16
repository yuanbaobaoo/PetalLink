package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.drive.DriveFile
import io.github.yuanbaobaao.petallink.sync.SyncAction
import io.github.yuanbaobaao.petallink.sync.SyncActionType
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class CycleCoordinatorTest {

    @Test
    fun request合并位集() {
        val cc = CycleCoordinator()
        cc.request(CycleRequest.LOCAL_RESCAN)
        cc.request(CycleRequest.CLOUD_FULL)
        // 两次 request 的位集合并到 pending（未 take 前）
        val (pending, _) = cc.takePending()
        assertTrue(pending.contains(CycleRequest.LOCAL_RESCAN))
        assertTrue(pending.contains(CycleRequest.CLOUD_FULL))
    }

    @Test
    fun takePending清空位集() {
        val cc = CycleCoordinator()
        cc.request(CycleRequest.LOCAL_RESCAN)
        val (pending1, _) = cc.takePending()
        assertFalse(pending1.isEmpty())
        val (pending2, _) = cc.takePending()
        assertTrue(pending2.isEmpty())
    }

    @Test
    fun complete后resultIfCompleted返回成功() {
        val cc = CycleCoordinator()
        val seq = cc.request(CycleRequest.LOCAL_RESCAN)
        cc.takePending()
        cc.complete(seq)
        val result = cc.resultIfCompleted(seq)
        assertNotNull(result)
        assertTrue(result.isSuccess)
    }

    @Test
    fun completeWithError后resultIfCompleted返回失败() {
        val cc = CycleCoordinator()
        val seq = cc.request(CycleRequest.LOCAL_RESCAN)
        cc.takePending()
        cc.complete(seq, "网络错误")
        val result = cc.resultIfCompleted(seq)
        assertNotNull(result)
        assertTrue(result.isFailure)
    }

    @Test
    fun 未完成返回null() {
        val cc = CycleCoordinator()
        val seq = cc.request(CycleRequest.LOCAL_RESCAN)
        cc.takePending()
        // 未 complete
        assertNull(cc.resultIfCompleted(seq))
    }

    @Test
    fun trigger映射正确() {
        assertEquals(
            CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_FULL,
            CycleTrigger.requestFor(CycleTrigger.MANUAL_REFRESH),
        )
        assertTrue(
            CycleTrigger.requestFor(CycleTrigger.STARTUP_RESUME)
                .contains(CycleRequest.STARTUP)
        )
    }
}

class ActivityTrackerTest {

    @Test
    fun 共享租约同路径可多个() {
        val tracker = ActivityTracker()
        val g1 = tracker.begin("a.txt")
        val g2 = tracker.begin("a.txt")
        assertNotNull(g1)
        assertNotNull(g2)
        assertEquals(2, tracker.activeCount())
        g1?.close()
        assertEquals(1, tracker.activeCount())
        g2?.close()
        assertEquals(0, tracker.activeCount())
    }

    @Test
    fun 独占租约与共享互斥() {
        val tracker = ActivityTracker()
        val shared = tracker.begin("a.txt")
        assertNotNull(shared)
        // 同路径独占应被拒绝
        assertNull(tracker.beginExclusive("a.txt"))
    }

    @Test
    fun 独占租约祖先后代互斥() {
        val tracker = ActivityTracker()
        val shared = tracker.begin("dir/sub.txt")
        assertNotNull(shared)
        // dir 是 dir/sub.txt 的祖先 → 独占 dir 应被拒绝
        assertNull(tracker.beginExclusive("dir"))
    }

    @Test
    fun close后拒绝新租约() {
        val tracker = ActivityTracker()
        tracker.close()
        assertNull(tracker.begin("a.txt"))
    }

    @Test
    fun syncPathsOverlap相等() {
        val tracker = ActivityTracker()
        assertTrue(tracker.syncPathsOverlap("a/b", "a/b"))
    }

    @Test
    fun syncPathsOverlap祖先后代() {
        val tracker = ActivityTracker()
        assertTrue(tracker.syncPathsOverlap("a/b/c", "a/b"))
        assertTrue(tracker.syncPathsOverlap("a/b", "a/b/c"))
    }

    @Test
    fun syncPathsOverlap不相关为false() {
        val tracker = ActivityTracker()
        assertFalse(tracker.syncPathsOverlap("a/b", "a/c"))
        assertFalse(tracker.syncPathsOverlap("abc", "ab"))
    }
}

class AntiOscillationTest {

    private fun action(type: SyncActionType, path: String) =
        SyncAction(type, path, "fid", null, "test")

    @Test
    fun 丢弃最近删除路径的动作() {
        val ao = AntiOscillation()
        ao.addDeleted("dir/file.txt", nowMs = 1000)
        val actions = listOf(
            action(SyncActionType.UPLOAD, "dir/file.txt"),  // 回弹 → 丢弃
            action(SyncActionType.UPLOAD, "other.txt"),     // 无关 → 保留
        )
        val filtered = ao.filter(actions)
        assertEquals(1, filtered.size)
        assertEquals("other.txt", filtered[0].relativePath)
    }

    @Test
    fun 保留DeleteFromCloud即使最近删除() {
        val ao = AntiOscillation()
        ao.addDeleted("dir/file.txt", nowMs = 1000)
        val actions = listOf(action(SyncActionType.DELETE_FROM_CLOUD, "dir/file.txt"))
        val filtered = ao.filter(actions)
        assertEquals(1, filtered.size)  // DeleteFromCloud 保留
    }

    @Test
    fun TTL过期后不再过滤() {
        val ao = AntiOscillation()
        ao.addDeleted("file.txt", nowMs = 1000)
        // 6 分钟后（超过 5 分钟 TTL）
        ao.purgeExpired(nowMs = 1000 + 6 * 60 * 1000)
        assertFalse(ao.contains("file.txt"))
    }

    @Test
    fun TTL未过期仍过滤() {
        val ao = AntiOscillation()
        ao.addDeleted("file.txt", nowMs = 1000)
        ao.purgeExpired(nowMs = 1000 + 4 * 60 * 1000)  // 4 分钟，未过期
        assertTrue(ao.contains("file.txt"))
    }
}

class StatusAggregatorTest {

    private val aggregator = StatusAggregator()

    @Test
    fun indexing状态优先() {
        val counts = StatusCounts(10, 0, 0, 0, 0, 0, 0)
        val runtime = RuntimeStatus(isIndexing = true)
        assertEquals(SyncGlobalStatus.INDEXING, aggregator.computeGlobalState(counts, runtime))
    }

    @Test
    fun 有上传下载时SYNCING() {
        val counts = StatusCounts(10, 0, 0, 1, 0, 0, 0)
        val runtime = RuntimeStatus()
        assertEquals(SyncGlobalStatus.SYNCING, aggregator.computeGlobalState(counts, runtime))
    }

    @Test
    fun 有失败时ERROR() {
        val counts = StatusCounts(10, 1, 0, 0, 0, 0, 0)
        val runtime = RuntimeStatus()
        assertEquals(SyncGlobalStatus.ERROR, aggregator.computeGlobalState(counts, runtime))
    }

    @Test
    fun 离线时PAUSED() {
        val counts = StatusCounts(10, 0, 0, 0, 0, 0, 0)
        val runtime = RuntimeStatus(isOnline = false)
        assertEquals(SyncGlobalStatus.PAUSED, aggregator.computeGlobalState(counts, runtime))
    }

    @Test
    fun 空闲IDLE() {
        val counts = StatusCounts(10, 0, 0, 0, 0, 0, 0)
        val runtime = RuntimeStatus()
        assertEquals(SyncGlobalStatus.IDLE, aggregator.computeGlobalState(counts, runtime))
    }

    @Test
    fun completed等于total减failed减conflict() {
        val counts = StatusCounts(100, 5, 3, 0, 0, 0, 0)
        assertEquals(92, counts.completed)
    }
}

class CloudTreeTest {

    private fun file(id: String, name: String = id) = DriveFile(id = id, name = name, mimeType = "text/plain")

    @Test
    fun validateTrusted_完整且一致_通过() {
        val tree = mapOf("a.txt" to file("fid1"), "dir" to file("fid2"))
        val pathToId = mapOf("a.txt" to "fid1", "dir" to "fid2")
        val cache = CloudTreeCache(tree, pathToId, "root", "cursor123", complete = true)
        cache.validateTrusted()  // 不抛异常
        assertTrue(cache.isTrusted())
    }

    @Test
    fun validateTrusted_不完整_失败() {
        val cache = CloudTreeCache(emptyMap(), emptyMap(), null, null, complete = false)
        assertFalse(cache.isTrusted())
    }

    @Test
    fun validateTrusted_无cursor_失败() {
        val cache = CloudTreeCache(emptyMap(), emptyMap(), null, cursor = null, complete = true)
        assertFalse(cache.isTrusted())
    }

    @Test
    fun validateTrusted_fileId重复_失败() {
        val tree = mapOf("a.txt" to file("dup"), "b.txt" to file("dup"))
        val pathToId = mapOf("a.txt" to "dup", "b.txt" to "dup")
        val cache = CloudTreeCache(tree, pathToId, null, "cursor", complete = true)
        assertFalse(cache.isTrusted())
    }

    @Test
    fun validateTrusted_pathToId不一致_失败() {
        val tree = mapOf("a.txt" to file("fid1"))
        val pathToId = mapOf("a.txt" to "wrong")  // 不一致
        val cache = CloudTreeCache(tree, pathToId, null, "cursor", complete = true)
        assertFalse(cache.isTrusted())
    }

    @Test
    fun detectRootFolderId_最高频parent() {
        val files = listOf(
            DriveFile(id = "f1", parent = "rootId"),
            DriveFile(id = "f2", parent = "rootId"),
            DriveFile(id = "f3", parent = "otherId"),
        )
        assertEquals("rootId", CloudTreeRefresh.detectRootFolderId(files))
    }

    @Test
    fun detectRootFolderId_平局返回null() {
        val files = listOf(
            DriveFile(id = "f1", parent = "a"),
            DriveFile(id = "f2", parent = "b"),
        )
        assertNull(CloudTreeRefresh.detectRootFolderId(files))
    }

    @Test
    fun validatePathSegment_合法通过() {
        CloudTreeRefresh.validatePathSegment("正常文件.txt")
    }

    @Test
    fun validatePathSegment_含斜杠失败() {
        var threw = false
        try { CloudTreeRefresh.validatePathSegment("a/b") } catch (e: IllegalArgumentException) { threw = true }
        assertTrue(threw)
    }

    @Test
    fun validatePathSegment_目录引用失败() {
        var threw = false
        try { CloudTreeRefresh.validatePathSegment("..") } catch (e: IllegalArgumentException) { threw = true }
        assertTrue(threw)
    }
}
