package io.github.yuanbaobaoo.petallink.sync.engine

import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class CoordinationFlowTest {
    @Test
    fun dispatcher合并所有来源且始终只有一个owner() = runBlocking {
        val coordinator = CycleCoordinator()
        val active = AtomicInteger()
        val maxActive = AtomicInteger()
        val dispatcher = CycleRequestDispatcher(this, coordinator) {
            val count = active.incrementAndGet()
            maxActive.updateAndGet { maxOf(it, count) }
            delay(5)
            active.decrementAndGet()
            Result.success(Unit)
        }
        val sequences = listOf(
            dispatcher.submit(CycleTrigger.MANUAL_REFRESH),
            dispatcher.submit(CycleTrigger.AUTO_CLOUD_REFRESH),
            dispatcher.submit(CycleTrigger.NETWORK_RECOVERY),
            dispatcher.submit(CycleTrigger.STARTUP_RESUME),
            dispatcher.submit(CycleTrigger.RETRY_FAILED),
        )
        withTimeout(2_000) {
            while (sequences.any { coordinator.resultIfCompleted(it) == null }) delay(5)
        }
        assertEquals(1, maxActive.get())
        assertTrue(sequences.all { coordinator.resultIfCompleted(it)?.isSuccess == true })
    }

    @Test
    fun 合并周期失败会覆盖本批所有序号() = runBlocking {
        val coordinator = CycleCoordinator()
        val first = coordinator.request(CycleRequest.LOCAL_RESCAN)
        val second = coordinator.request(CycleRequest.CLOUD_FULL)
        coordinator.drainOwned { Result.failure(IllegalStateException("failed")) }
        assertTrue(coordinator.resultIfCompleted(first)?.isFailure == true)
        assertTrue(coordinator.resultIfCompleted(second)?.isFailure == true)
    }

    @Test
    fun activityShutdown封门后拒绝新动作并等待旧guard() = runBlocking {
        val tracker = ActivityTracker()
        val guard = tracker.begin("a.txt")!!
        val closing = async { tracker.closeAndWait() }
        delay(10)
        assertFalse(closing.isCompleted)
        assertNull(tracker.begin("b.txt"))
        guard.close()
        closing.await()
        assertEquals(0, tracker.activeCount())
        guard.close()
        assertEquals(0, tracker.activeCount(), "guard 重复 close 必须幂等")
    }

    @Test
    fun 文件夹_传输_上传失败边沿事件可订阅() = runBlocking {
        val hub = SyncEventHub()
        val folder = async(start = CoroutineStart.UNDISPATCHED) { hub.folderChanges.first() }
        val transfer = async(start = CoroutineStart.UNDISPATCHED) { hub.transferUpdates.first() }
        val failed = async(start = CoroutineStart.UNDISPATCHED) { hub.uploadFailures.first() }
        hub.publishFolderChange(FolderChangeEvent(listOf("a"), false))
        hub.publishTransferUpdate(TransferUpdateEvent(1, 2))
        hub.publishUploadFailed(UploadFailedEvent("a", "failed"))
        assertEquals(listOf("a"), folder.await().paths)
        assertEquals(2, transfer.await().revision)
        assertEquals("failed", failed.await().message)
    }
}
