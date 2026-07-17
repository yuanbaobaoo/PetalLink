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
 * 同步状态 ViewModel（对标原项目前端 sync store）。
 *
 * 详见 docs/08 §sync store。
 * applyState 同 revision 重复投递只幂等赋值，不触发 sidebarRefresh++。
 */
class SyncViewModel {
    private val _state = MutableStateFlow(SyncGlobalState.IDLE)
    val state: StateFlow<SyncGlobalState> = _state.asStateFlow()

    private val _netState = MutableStateFlow(NetState.OFFLINE)
    val netState: StateFlow<NetState> = _netState.asStateFlow()

    private val _lastSyncTime = MutableStateFlow(0L)
    val lastSyncTime: StateFlow<Long> = _lastSyncTime.asStateFlow()

    private var lastRevision: Long = -1

    /**
     * 应用同步状态（isNewRevision 逻辑：同 revision 重复投递只幂等赋值）。
     */
    fun applyState(newState: SyncGlobalState, revision: Long) {
        if (revision <= lastRevision) return
        lastRevision = revision
        _state.value = newState
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
