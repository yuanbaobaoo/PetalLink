package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.data.PetalLinkDb
import io.github.yuanbaobaao.petallink.sync.TransferState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.concurrent.atomic.AtomicLong

/**
 * 同步状态聚合器（对标 src/sync/status_aggregator.rs）。
 * snapshot 执行真实 7 子查询 SQL。
 */
class StatusAggregator {
    private val nextRevision = AtomicLong(0L)
    private val _currentState = MutableStateFlow(SyncGlobalStatus.IDLE)
    val currentState: StateFlow<SyncGlobalStatus> = _currentState.asStateFlow()

    /** 真实 DB 查询（7 子查询） */
    suspend fun snapshot(db: PetalLinkDb) {
        val total = db.syncItems.countAll()
        val failed = db.syncItems.countByStatus(8)
        val conflict = db.syncItems.countByStatus(2)
        val uploading = db.transfers.countByStateAndDirection(TransferState.Running, 0)
        val downloading = db.transfers.countByStateAndDirection(TransferState.Running, 1)
        val waitingNetwork = db.transfers.countByState(TransferState.WaitingForNetwork)
        val transferFailed = db.transfers.countByState(TransferState.Failed)

        nextRevision.addAndGet(1L)
        val counts = StatusCounts(total, failed, conflict, uploading, downloading, waitingNetwork, transferFailed)
        _currentState.value = computeGlobalState(counts, RuntimeStatus(isRunning = true))
    }

    fun computeGlobalState(counts: StatusCounts, runtime: RuntimeStatus): SyncGlobalStatus {
        if (runtime.isIndexing) return SyncGlobalStatus.INDEXING
        if (counts.uploading > 0 || counts.downloading > 0) return SyncGlobalStatus.SYNCING
        if (counts.failed > 0 || counts.transferFailed > 0) return SyncGlobalStatus.ERROR
        if (!runtime.isOnline) return SyncGlobalStatus.PAUSED
        return SyncGlobalStatus.IDLE
    }
}

data class StatusCounts(
    val total: Long, val failed: Long, val conflict: Long,
    val uploading: Long, val downloading: Long,
    val waitingNetwork: Long, val transferFailed: Long,
) { val completed: Long get() = total - failed - conflict }

data class RuntimeStatus(
    val isRunning: Boolean = false, val isIndexing: Boolean = false,
    val isOnline: Boolean = true, val lastSyncTime: Long = 0L,
    val indexingScannedFolders: Int = 0, val indexingDiscoveredItems: Int = 0,
    val contentChanged: Boolean = false,
)

enum class SyncGlobalStatus { IDLE, INDEXING, SYNCING, PAUSED, ERROR }
