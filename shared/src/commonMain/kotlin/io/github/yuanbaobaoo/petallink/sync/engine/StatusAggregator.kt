package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.sync.SyncStatus
import io.github.yuanbaobaoo.petallink.sync.TransferState
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow

data class FailedSyncItem(val relativePath: String, val errorMessage: String?)

data class SyncStatusSnapshot(
    val revision: Long,
    val global: SyncGlobalStatus,
    val counts: StatusCounts,
    val runtime: RuntimeStatus,
    val failedItems: List<FailedSyncItem>,
)

/** 一个版本号对应一份完整、不可变的 DB + runtime 快照。 */
class StatusAggregator {
    private val nextRevision = AtomicLong(0L)
    private val initialCounts = StatusCounts(0, 0, 0, 0, 0, 0, 0)
    private val initial = SyncStatusSnapshot(0, SyncGlobalStatus.IDLE, initialCounts, RuntimeStatus(), emptyList())
    private val mutableSnapshot = MutableStateFlow(initial)
    val snapshots: StateFlow<SyncStatusSnapshot> = mutableSnapshot.asStateFlow()
    private val mutableState = MutableStateFlow(SyncGlobalStatus.IDLE)
    val currentState: StateFlow<SyncGlobalStatus> = mutableState.asStateFlow()

    suspend fun snapshot(db: PetalLinkDb, runtime: RuntimeStatus = RuntimeStatus()): SyncStatusSnapshot {
        val total = db.syncItems.countAll()
        val failed = db.syncItems.countByStatus(SyncStatus.FAILED)
        val conflict = db.syncItems.countByStatus(SyncStatus.CONFLICT)
        val uploading = db.transfers.countByStateAndDirection(TransferState.Running, 0)
        val downloading =
            db.transfers.countByStateAndDirection(TransferState.Running, 1) +
                db.transfers.countByStateAndDirection(TransferState.Running, 3)
        val waitingNetwork = db.transfers.countByState(TransferState.WaitingForNetwork)
        val transferFailed = db.transfers.countByState(TransferState.Failed)
        val failedItems = db.syncItems.selectByStatus(SyncStatus.FAILED).take(20)
            .map { FailedSyncItem(it.localPath, it.errorMessage) }
        val counts = StatusCounts(total, failed, conflict, uploading, downloading, waitingNetwork, transferFailed)
        val result = SyncStatusSnapshot(
            revision = nextRevision.incrementAndGet(),
            global = computeGlobalState(counts, runtime),
            counts = counts,
            runtime = runtime,
            failedItems = failedItems,
        )
        mutableSnapshot.value = result
        mutableState.value = result.global
        return result
    }

    fun computeGlobalState(counts: StatusCounts, runtime: RuntimeStatus): SyncGlobalStatus {
        if (runtime.isIndexing) return SyncGlobalStatus.INDEXING
        if (counts.uploading > 0 || counts.downloading > 0 || runtime.isRunning) return SyncGlobalStatus.SYNCING
        if (counts.failed > 0 || counts.transferFailed > 0) return SyncGlobalStatus.ERROR
        if (!runtime.isOnline) return SyncGlobalStatus.PAUSED
        return SyncGlobalStatus.IDLE
    }
}

data class StatusCounts(
    val total: Long, val failed: Long, val conflict: Long,
    val uploading: Long, val downloading: Long,
    val waitingNetwork: Long, val transferFailed: Long,
) { val completed: Long get() = (total - failed - conflict).coerceAtLeast(0) }

data class RuntimeStatus(
    val isRunning: Boolean = false, val isIndexing: Boolean = false,
    val isOnline: Boolean = true, val lastSyncTime: Long? = null,
    val editing: Long = 0L,
    val indexingScannedFolders: Int = 0, val indexingDiscoveredItems: Int = 0,
    val contentChanged: Boolean = false, val syncPhase: String? = null,
)

enum class SyncGlobalStatus { IDLE, INDEXING, SYNCING, PAUSED, ERROR }

data class FolderChangeEvent(val paths: List<String>, val fullRescan: Boolean)
data class TransferUpdateEvent(val taskId: Long?, val revision: Long)
data class UploadFailedEvent(val relativePath: String, val message: String)

/** 内部事件总线：状态用 StateFlow，边沿通知用 SharedFlow。 */
class SyncEventHub(val status: StatusAggregator = StatusAggregator()) {
    val syncState: StateFlow<SyncStatusSnapshot> = status.snapshots
    private val mutableFolderChanges = MutableSharedFlow<FolderChangeEvent>(extraBufferCapacity = 64)
    private val mutableTransferUpdates = MutableSharedFlow<TransferUpdateEvent>(extraBufferCapacity = 128)
    private val mutableUploadFailures = MutableSharedFlow<UploadFailedEvent>(extraBufferCapacity = 64)
    val folderChanges: SharedFlow<FolderChangeEvent> = mutableFolderChanges.asSharedFlow()
    val transferUpdates: SharedFlow<TransferUpdateEvent> = mutableTransferUpdates.asSharedFlow()
    val uploadFailures: SharedFlow<UploadFailedEvent> = mutableUploadFailures.asSharedFlow()

    suspend fun publishFolderChange(event: FolderChangeEvent) = mutableFolderChanges.emit(event)
    suspend fun publishTransferUpdate(event: TransferUpdateEvent) = mutableTransferUpdates.emit(event)
    suspend fun publishUploadFailed(event: UploadFailedEvent) = mutableUploadFailures.emit(event)
}
