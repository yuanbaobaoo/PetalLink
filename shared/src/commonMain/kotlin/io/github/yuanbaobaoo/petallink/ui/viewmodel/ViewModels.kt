package io.github.yuanbaobaoo.petallink.ui.viewmodel

import io.github.yuanbaobaoo.petallink.auth.TokenPair
import io.github.yuanbaobaoo.petallink.core.net_guard.NetState
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.sync.isFolder
import io.github.yuanbaobaoo.petallink.sync.TransferState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * 认证 ViewModel（对标原项目前端 auth store）。
 *
 * 详见 docs/08 §auth store、docs/10 阶段 6 item 32。
 */
class AuthViewModel {
    private val _isLoggedIn = MutableStateFlow(false)
    val isLoggedIn: StateFlow<Boolean> = _isLoggedIn.asStateFlow()

    private val _userInfo = MutableStateFlow<UserInfo?>(null)
    val userInfo: StateFlow<UserInfo?> = _userInfo.asStateFlow()

    fun onLoginSuccess(token: TokenPair, info: UserInfo) {
        _isLoggedIn.value = true
        _userInfo.value = info
    }

    fun onLogout() {
        _isLoggedIn.value = false
        _userInfo.value = null
    }
}

/** 用户信息（对标 UserInfo） */
data class UserInfo(
    val displayName: String?,
    val nickname: String?,
    val mobile: String?,
    val avatarUrl: String?,
)

/**
 * 同步快照 UI 模型（对标原 Vue sync store 全部字段 + docs/08 §sync store）。
 *
 * 字段对齐原 Rust `SyncGlobalState`（src/sync/state.rs）与 `RuntimeStatus`。
 * SyncStatusBar 按 [syncPhase] 9 种值映射精确文案，failedItems 驱动失败弹窗。
 */
data class SyncSnapshotUi(
    val revision: Long = 0L,
    val global: SyncGlobalState = SyncGlobalState.IDLE,
    // 计数器（对标 StatusCounts）
    val total: Long = 0L,
    val completed: Long = 0L,
    val uploading: Long = 0L,
    val downloading: Long = 0L,
    val waitingNetwork: Long = 0L,
    val failed: Long = 0L,
    val transferFailed: Long = 0L,
    val conflict: Long = 0L,
    val editing: Long = 0L,
    // 运行态（对标 RuntimeStatus）
    val isRunning: Boolean = false,
    val isIndexing: Boolean = false,
    val indexingScannedFolders: Long = 0L,
    val indexingDiscoveredItems: Long = 0L,
    val syncPhase: String? = null,
    val lastSyncTime: Long? = null,
    val contentChanged: Boolean = false,
    // 失败项详情（最多 20 条）
    val failedItems: List<FailedItemUi> = emptyList(),
) {
    /** 进度 0..1（completed/total，total=0 视为 1.0）。 */
    val progress: Float get() = if (total == 0L) 1f else (completed.toFloat() / total).coerceIn(0f, 1f)
    /** 是否有活跃传输（上传/下载/等待网络任一 >0）。 */
    val hasActiveTransfer: Boolean get() = uploading > 0 || downloading > 0 || waitingNetwork > 0
    /** 是否空闲（无活跃传输、非索引、非运行）。 */
    val isIdle: Boolean get() = !hasActiveTransfer && !isIndexing && !isRunning
}

/** 失败项 UI 模型（对标原 Vue FailedItem）。 */
data class FailedItemUi(
    val relativePath: String,
    val errorMessage: String?,
)

/** 同步目录配置阶段（对标原 Vue sync.setupPhase）。 */
enum class SetupPhase { LOADING, NEEDS_SETUP, NEEDS_FIRST_SYNC, ACTIVE }

/**
 * 同步状态 ViewModel（对标原项目前端 sync store）。
 *
 * 详见 docs/08 §sync store。
 * applyState/applySnapshot 同 revision 重复投递只幂等赋值，不触发 sidebarRefresh++。
 * sidebarRefresh 是 contentChanged 同新 revision 时 +1 的计数器，供目录树订阅刷新。
 */
class SyncViewModel {
    private val _state = MutableStateFlow(SyncGlobalState.IDLE)
    val state: StateFlow<SyncGlobalState> = _state.asStateFlow()

    private val _snapshot = MutableStateFlow(SyncSnapshotUi())
    val snapshot: StateFlow<SyncSnapshotUi> = _snapshot.asStateFlow()

    private val _netState = MutableStateFlow(NetState.OFFLINE)
    val netState: StateFlow<NetState> = _netState.asStateFlow()

    private val _lastSyncTime = MutableStateFlow(0L)
    val lastSyncTime: StateFlow<Long> = _lastSyncTime.asStateFlow()

    /** 目录树刷新计数器（contentChanged 同新 revision 时 +1，对标原 Vue sidebarRefresh）。 */
    private val _sidebarRefresh = MutableStateFlow(0)
    val sidebarRefresh: StateFlow<Int> = _sidebarRefresh.asStateFlow()

    private var lastRevision: Long = -1

    /**
     * 应用同步状态（isNewRevision 逻辑：同 revision 重复投递只幂等赋值）。
     * 兼容旧调用方；完整快照用 [applySnapshot]。
     */
    fun applyState(newState: SyncGlobalState, revision: Long) {
        if (revision <= lastRevision) return
        lastRevision = revision
        _state.value = newState
        _snapshot.value = _snapshot.value.copy(revision = revision, global = newState)
    }

    /**
     * 应用完整快照（乱序保护：revision <= lastRevision 拒绝）。
     *
     * contentChanged 且为新 revision 时 sidebarRefresh++（目录树据此刷新），
     * 同 revision 重复投递只幂等赋值，不重复触发刷新。
     */
    fun applySnapshot(snap: SyncSnapshotUi) {
        if (snap.revision <= lastRevision) return
        val isNewRevision = snap.revision > lastRevision
        lastRevision = snap.revision
        _state.value = snap.global
        _snapshot.value = snap
        _lastSyncTime.value = snap.lastSyncTime ?: 0L
        if (snap.contentChanged && isNewRevision) {
            _sidebarRefresh.value = _sidebarRefresh.value + 1
        }
    }

    fun updateNetState(state: NetState) {
        _netState.value = state
    }
}

/** 同步全局状态（对标 SyncGlobalState） */
enum class SyncGlobalState { IDLE, INDEXING, SYNCING, PAUSED, ERROR }

/**
 * 传输 ViewModel（对标原项目前端 transfer store）。
 *
 * 详见 docs/08 §transfer store。
 * loadAll 两重乱序保护：requestId 递增 + per-task state_revision 比对。
 */
class TransferViewModel {
    private val _tasks = MutableStateFlow<List<TransferTaskUi>>(emptyList())
    val tasks: StateFlow<List<TransferTaskUi>> = _tasks.asStateFlow()

    private var loadRequestId = 0

    /**
     * 加载任务列表（两重乱序保护）。
     * @param requestId 本次请求 ID（递增），过期请求的结果被丢弃
     */
    fun loadAll(newTasks: List<TransferTaskUi>, requestId: Int) {
        if (requestId < loadRequestId) return  // 过期请求丢弃
        loadRequestId = requestId
        val revisions = _tasks.value.associateBy { it.id }
        _tasks.value = newTasks.map { incoming ->
            revisions[incoming.id]?.takeIf { it.stateRevision > incoming.stateRevision } ?: incoming
        }
    }

    /** 更新单个任务进度（乱序保护：revision 比对） */
    fun updateProgress(taskId: Long, bytesDone: Long, revision: Long) {
        val current = _tasks.value
        val updated = current.map { task ->
            if (task.id == taskId && revision >= task.stateRevision) {
                task.copy(bytesDone = bytesDone, stateRevision = revision)
            } else task
        }
        _tasks.value = updated
    }
}

enum class BrowserSortField { NAME, SIZE, MODIFIED_TIME }

data class BrowserBreadcrumb(val id: String?, val name: String)

data class FileBrowserState(
    val folderId: String? = null,
    val breadcrumbs: List<BrowserBreadcrumb> = listOf(BrowserBreadcrumb(null, "全部文件")),
    val files: List<DriveFile> = emptyList(),
    val nextCursor: String? = null,
    val query: String = "",
    val sortField: BrowserSortField = BrowserSortField.NAME,
    val ascending: Boolean = true,
    val loading: Boolean = false,
    val directoryChildren: Map<String, List<DriveFile>> = emptyMap(),
) {
    val visibleFiles: List<DriveFile> get() {
        val filtered = if (query.isBlank()) files else files.filter {
            (it.name ?: it.fileName.orEmpty()).contains(query, ignoreCase = true)
        }
        val itemComparator = Comparator<DriveFile> { left, right ->
            val result = when (sortField) {
                BrowserSortField.NAME -> displayName(left).compareTo(displayName(right), ignoreCase = true)
                BrowserSortField.SIZE -> left.sizeBytes.compareTo(right.sizeBytes)
                BrowserSortField.MODIFIED_TIME -> left.modifiedTime.orEmpty().compareTo(right.modifiedTime.orEmpty())
            }
            if (ascending) result else -result
        }
        return filtered.sortedWith(compareByDescending<DriveFile> { it.isFolder() }.then(itemComparator))
    }
}

/** 分页、路径和目录树写入都由 requestId 保护的纯状态模型。 */
class FileBrowserViewModel {
    private val _state = MutableStateFlow(FileBrowserState())
    val state: StateFlow<FileBrowserState> = _state.asStateFlow()
    private var requestSequence = 0L
    private var acceptedRequest = 0L
    private val childrenByFolder = mutableMapOf<String?, List<DriveFile>>()

    fun beginLoad(): Long {
        val request = ++requestSequence
        _state.value = _state.value.copy(loading = true)
        return request
    }

    fun applyPage(
        requestId: Long,
        folderId: String?,
        files: List<DriveFile>,
        nextCursor: String?,
        append: Boolean,
    ): Boolean {
        if (requestId < acceptedRequest || folderId != _state.value.folderId) return false
        acceptedRequest = requestId
        val merged = if (append) (_state.value.files + files).distinctBy { it.id } else files
        childrenByFolder[folderId] = merged.filter { it.isFolder() }
        val key = folderId ?: ROOT_KEY
        _state.value = _state.value.copy(
            files = merged,
            nextCursor = nextCursor,
            loading = false,
            directoryChildren = _state.value.directoryChildren + (key to merged.filter { it.isFolder() }),
        )
        return true
    }

    fun enter(folder: DriveFile) {
        require(folder.isFolder()) { "只能进入文件夹" }
        val id = requireNotNull(folder.id) { "文件夹缺少 id" }
        acceptedRequest = ++requestSequence
        _state.value = _state.value.copy(
            folderId = id,
            breadcrumbs = _state.value.breadcrumbs + BrowserBreadcrumb(id, displayName(folder)),
            files = emptyList(), nextCursor = null, query = "", loading = true,
        )
    }

    fun navigateTo(breadcrumb: BrowserBreadcrumb) {
        acceptedRequest = ++requestSequence
        val index = _state.value.breadcrumbs.indexOfFirst { it.id == breadcrumb.id }
        val path = if (index >= 0) _state.value.breadcrumbs.take(index + 1) else listOf(breadcrumb)
        _state.value = _state.value.copy(
            folderId = breadcrumb.id, breadcrumbs = path, files = emptyList(),
            nextCursor = null, query = "", loading = true,
        )
    }

    fun search(query: String) { _state.value = _state.value.copy(query = query) }

    fun sort(field: BrowserSortField) {
        val current = _state.value
        _state.value = current.copy(
            sortField = field,
            ascending = if (field == current.sortField) !current.ascending else true,
        )
    }

    fun treeChildren(folderId: String?): List<DriveFile> = childrenByFolder[folderId].orEmpty()

    companion object { const val ROOT_KEY = "__root__" }
}

private fun displayName(file: DriveFile): String = file.name ?: file.fileName ?: "未命名"

/** 传输任务 UI 模型 */
data class TransferTaskUi(
    val id: Long,
    val fileName: String,
    val state: TransferState,
    val stateRevision: Long,
    val bytesTotal: Long,
    val bytesDone: Long,
    val direction: String,  // "upload" / "download"
    val errorMessage: String?,
) {
    /** 进度百分比 0..1 */
    val progress: Float get() = if (bytesTotal == 0L) 0f else bytesDone.toFloat() / bytesTotal
}

/**
 * 更新器 ViewModel（对标原项目前端 updater store）。
 *
 * 详见 docs/08 §updater store、docs/10 阶段 6。
 * 9 phase + waitForTransfers 轮询最多 5min 每 2s。
 */
class UpdaterViewModel {
    private val _phase = MutableStateFlow(UpdaterPhase.IDLE)
    val phase: StateFlow<UpdaterPhase> = _phase.asStateFlow()

    private val _newVersion = MutableStateFlow<String?>(null)
    val newVersion: StateFlow<String?> = _newVersion.asStateFlow()

    fun setPhase(phase: UpdaterPhase) {
        _phase.value = phase
    }

    fun setNewVersion(version: String?) {
        _newVersion.value = version
    }

    companion object {
        /** waitForTransfers 最大等待 5 分钟，每 2 秒轮询 */
        const val WAIT_TIMEOUT_MS = 300_000L
        const val WAIT_POLL_INTERVAL_MS = 2_000L
    }
}

/** 更新器 9 phase（对标原项目 updater 状态机） */
enum class UpdaterPhase {
    IDLE,              // 空闲
    CHECKING,          // 检查更新中
    AVAILABLE,         // 有新版本
    WAITING_TRANSFERS, // 等待传输完成
    DOWNLOADING,       // 下载更新包
    VERIFYING,         // 验证签名
    READY,             // 准备就绪
    INSTALLING,        // 安装中
    FAILED,            // 失败
}
