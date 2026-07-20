package io.github.yuanbaobaoo.petallink.ui.viewmodel

import io.github.yuanbaobaoo.petallink.config.SortField
import io.github.yuanbaobaoo.petallink.core.net_guard.NetState
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.drive.displayName
import io.github.yuanbaobaoo.petallink.sync.engine.FailedSyncItem
import io.github.yuanbaobaoo.petallink.sync.engine.SyncGlobalStatus
import io.github.yuanbaobaoo.petallink.sync.isFolder
import io.github.yuanbaobaoo.petallink.sync.TransferState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * 同步快照 UI 模型（对标原 Vue sync store 全部字段 + docs/08 §sync store）。
 *
 * 字段对齐原 Rust `SyncGlobalState`（src/sync/state.rs）与 `RuntimeStatus`。
 * SyncStatusBar 按 [syncPhase] 9 种值映射精确文案，failedItems 驱动失败弹窗。
 *
 * 复用后端 [SyncGlobalStatus] / [FailedSyncItem]，避免前端另起一套枚举与模型。
 */
data class SyncSnapshotUi(
    val revision: Long = 0L,
    val global: SyncGlobalStatus = SyncGlobalStatus.IDLE,
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
    val failedItems: List<FailedSyncItem> = emptyList(),
) {
    /**
     * 进度 0..1（completed/total，total=0 视为 1.0）。
     */
    val progress: Float get() = if (total == 0L) 1f else (completed.toFloat() / total).coerceIn(0f, 1f)

    /**
     * 是否有活跃传输（上传/下载/等待网络任一 >0）。
     */
    val hasActiveTransfer: Boolean get() = uploading > 0 || downloading > 0 || waitingNetwork > 0

    /**
     * 是否空闲（无活跃传输、非索引、非运行）。
     */
    val isIdle: Boolean get() = !hasActiveTransfer && !isIndexing && !isRunning
}

/**
 * 失败项 UI 模型（对标原 Vue FailedItem）。
 *
 * 直接复用后端 [io.github.yuanbaobaoo.petallink.sync.engine.FailedSyncItem]，
 * 不再单独定义 UI 版本。
 */

/**
 * 同步目录配置阶段（对标原 Vue sync.setupPhase）。
 */
enum class SetupPhase { LOADING, NEEDS_SETUP, NEEDS_FIRST_SYNC, ACTIVE }

/**
 * 同步状态 ViewModel（对标原项目前端 sync store）。
 *
 * 详见 docs/08 §sync store。
 * applyState/applySnapshot 同 revision 重复投递只幂等赋值，不触发 sidebarRefresh++。
 * sidebarRefresh 是 contentChanged 同新 revision 时 +1 的计数器，供目录树订阅刷新。
 */
class SyncViewModel {
    private val _state = MutableStateFlow(SyncGlobalStatus.IDLE)
    val state: StateFlow<SyncGlobalStatus> = _state.asStateFlow()

    private val _snapshot = MutableStateFlow(SyncSnapshotUi())
    val snapshot: StateFlow<SyncSnapshotUi> = _snapshot.asStateFlow()

    private val _netState = MutableStateFlow(NetState.OFFLINE)
    val netState: StateFlow<NetState> = _netState.asStateFlow()

    private val _lastSyncTime = MutableStateFlow(0L)
    val lastSyncTime: StateFlow<Long> = _lastSyncTime.asStateFlow()

    /**
     * 目录树刷新计数器（contentChanged 同新 revision 时 +1，对标原 Vue sidebarRefresh）。
     */
    private val _sidebarRefresh = MutableStateFlow(0)
    val sidebarRefresh: StateFlow<Int> = _sidebarRefresh.asStateFlow()

    private var lastRevision: Long = -1

    /**
     * 应用同步状态（isNewRevision 逻辑：同 revision 重复投递只幂等赋值）。
     * 兼容旧调用方；完整快照用 [applySnapshot]。
     */
    fun applyState(newState: SyncGlobalStatus, revision: Long) {
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

    /**
     * 更新网络状态（供 UI 据此显示在线/离线提示）
     */
    fun updateNetState(state: NetState) {
        _netState.value = state
    }
}

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

    /**
     * 更新单个任务进度（乱序保护：revision 比对）
     */
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

/**
 * 面包屑导航节点，表示当前路径中的一级目录
 */
data class BrowserBreadcrumb(val id: String?, val name: String)

/**
 * 文件浏览器整体状态，包含当前文件夹、面包屑、文件列表及排序/筛选等
 *
 * 排序字段直接复用后端 [SortField]（含持久化注解），不再单独定义 UI 版枚举。
 */
data class FileBrowserState(
    val folderId: String? = null,
    val breadcrumbs: List<BrowserBreadcrumb> = listOf(BrowserBreadcrumb(null, "全部文件")),
    val files: List<DriveFile> = emptyList(),
    val nextCursor: String? = null,
    val query: String = "",
    val sortField: SortField = SortField.Name,
    val ascending: Boolean = true,
    val loading: Boolean = false,
    val directoryChildren: Map<String, List<DriveFile>> = emptyMap(),
    val treeLoadingIds: Set<String> = emptySet(),
) {
    val visibleFiles: List<DriveFile> get() {
        val filtered = if (query.isBlank()) files else files.filter {
            (it.name ?: it.fileName.orEmpty()).contains(query, ignoreCase = true)
        }
        val itemComparator = Comparator<DriveFile> { left, right ->
            val result = when (sortField) {
                SortField.Name -> left.displayName().compareTo(right.displayName(), ignoreCase = true)
                SortField.Size -> left.sizeBytes.compareTo(right.sizeBytes)
                SortField.ModifiedTime -> left.modifiedTime.orEmpty().compareTo(right.modifiedTime.orEmpty())
            }
            if (ascending) result else -result
        }
        return filtered.sortedWith(compareByDescending<DriveFile> { it.isFolder() }.then(itemComparator))
    }
}

/**
 * 分页、路径和目录树写入都由 requestId 保护的纯状态模型。
 */
class FileBrowserViewModel {
    private val _state = MutableStateFlow(FileBrowserState())
    val state: StateFlow<FileBrowserState> = _state.asStateFlow()
    private var requestSequence = 0L
    private var acceptedRequest = 0L
    private val childrenByFolder = mutableMapOf<String?, List<DriveFile>>()

    /**
     * 开始一次加载：递增请求序列并进入 loading 态，返回本次请求 ID
     */
    fun beginLoad(): Long {
        val request = ++requestSequence
        _state.value = _state.value.copy(loading = true)
        return request
    }

    /**
     * 应用一页文件结果（乱序保护：过期请求或不同目录被拒绝）。
     * @return 是否被接受
     */
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

    /**
     * 进入指定文件夹：切换 folderId、追加面包屑并进入 loading 态
     */
    fun enter(folder: DriveFile) {
        require(folder.isFolder()) { "只能进入文件夹" }
        val id = requireNotNull(folder.id) { "文件夹缺少 id" }
        acceptedRequest = ++requestSequence
        _state.value = _state.value.copy(
            folderId = id,
            breadcrumbs = _state.value.breadcrumbs + BrowserBreadcrumb(id, folder.displayName()),
            files = emptyList(), nextCursor = null, query = "", loading = true,
        )
    }

    /**
     * 跳转到面包屑节点：截断路径至该节点并进入 loading 态
     */
    fun navigateTo(breadcrumb: BrowserBreadcrumb) {
        acceptedRequest = ++requestSequence
        val index = _state.value.breadcrumbs.indexOfFirst { it.id == breadcrumb.id }
        val path = if (index >= 0) _state.value.breadcrumbs.take(index + 1) else listOf(breadcrumb)
        _state.value = _state.value.copy(
            folderId = breadcrumb.id, breadcrumbs = path, files = emptyList(),
            nextCursor = null, query = "", loading = true,
        )
    }

    /**
     * 设置搜索关键词，用于对当前文件列表做本地过滤
     */
    fun search(query: String) { _state.value = _state.value.copy(query = query) }

    /**
     * 切换排序字段；同字段再次点击则反转升降序
     */
    fun sort(field: SortField) {
        val current = _state.value
        _state.value = current.copy(
            sortField = field,
            ascending = if (field == current.sortField) !current.ascending else true,
        )
    }

    /**
     * 返回指定文件夹下已缓存的子目录，供目录树懒加载展示
     */
    fun treeChildren(folderId: String?): List<DriveFile> = childrenByFolder[folderId].orEmpty()

    /**
     * 开始一次目录树懒加载：标记节点 loading（不参与文件列表的 requestId 乱序保护）。
     */
    fun beginTreeLoad(folderId: String) {
        _state.value = _state.value.copy(treeLoadingIds = _state.value.treeLoadingIds + folderId)
    }

    /**
     * 写入目录树懒加载结果：只更新目录树缓存，不改变当前浏览位置。
     */
    fun applyTreeChildren(folderId: String, files: List<DriveFile>) {
        val folders = files.filter { it.isFolder() }
        childrenByFolder[folderId] = folders
        _state.value = _state.value.copy(
            directoryChildren = _state.value.directoryChildren + (folderId to folders),
            treeLoadingIds = _state.value.treeLoadingIds - folderId,
        )
    }

    /**
     * 目录树懒加载失败：清除节点 loading，保留旧缓存，用户可重新展开重试。
     */
    fun failTreeLoad(folderId: String) {
        _state.value = _state.value.copy(treeLoadingIds = _state.value.treeLoadingIds - folderId)
    }

    /**
     * 计算目录树节点从根到自身的完整路径（用于树点击替换面包屑，对标原 Vue SidebarTreeNode 的 path prop）。
     */
    fun treePathTo(folderId: String?): List<BrowserBreadcrumb> {
        val root = listOf(BrowserBreadcrumb(null, "全部文件"))
        if (folderId == null) return root
        val names = mutableMapOf<String, String>()
        val parentOf = mutableMapOf<String, String?>()
        for ((parentId, children) in _state.value.directoryChildren) {
            for (child in children) {
                val id = child.id ?: continue
                names[id] = child.displayName()
                parentOf[id] = parentId.takeIf { it != ROOT_KEY }
            }
        }
        val segments = mutableListOf<BrowserBreadcrumb>()
        var current: String? = folderId
        var guard = 0
        while (current != null && guard++ < 100) {
            val name = names[current] ?: break
            segments += BrowserBreadcrumb(current, name)
            current = parentOf[current]
        }
        return root + segments.asReversed()
    }

    /**
     * 用目录树给定的完整路径替换面包屑并进入该文件夹（对标原 Vue 树点击 pathStack 替换）。
     */
    fun enterWithPath(folder: DriveFile, path: List<BrowserBreadcrumb>) {
        require(folder.isFolder()) { "只能进入文件夹" }
        val id = requireNotNull(folder.id) { "文件夹缺少 id" }
        acceptedRequest = ++requestSequence
        _state.value = _state.value.copy(
            folderId = id, breadcrumbs = path, files = emptyList(),
            nextCursor = null, query = "", loading = true,
        )
    }

    companion object { const val ROOT_KEY = "__root__" }
}

/**
 * 传输任务 UI 模型
 */
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
    /**
     * 进度百分比 0..1
     */
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

    /**
     * 设置当前更新阶段
     */
    fun setPhase(phase: UpdaterPhase) {
        _phase.value = phase
    }

    /**
     * 设置检测到的新版本号（null 表示无新版本）
     */
    fun setNewVersion(version: String?) {
        _newVersion.value = version
    }

    companion object {
        /**
         * waitForTransfers 最大等待 5 分钟，每 2 秒轮询
         */
        const val WAIT_TIMEOUT_MS = 300_000L
        const val WAIT_POLL_INTERVAL_MS = 2_000L
    }
}

/**
 * 更新器 9 phase（对标原项目 updater 状态机）
 */
enum class UpdaterPhase {
    /**
     * 空闲
     */
    IDLE,

    /**
     * 检查更新中
     */
    CHECKING,

    /**
     * 有新版本
     */
    AVAILABLE,

    /**
     * 等待传输完成
     */
    WAITING_TRANSFERS,

    /**
     * 下载更新包
     */
    DOWNLOADING,

    /**
     * 验证签名
     */
    VERIFYING,

    /**
     * 准备就绪
     */
    READY,

    /**
     * 安装中
     */
    INSTALLING,

    /**
     * 失败
     */
    FAILED,
}
