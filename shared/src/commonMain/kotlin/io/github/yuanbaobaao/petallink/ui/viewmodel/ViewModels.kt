package io.github.yuanbaobaao.petallink.ui.viewmodel

import io.github.yuanbaobaao.petallink.auth.TokenPair
import io.github.yuanbaobaao.petallink.core.net_guard.NetState
import io.github.yuanbaobaao.petallink.sync.TransferState
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
        if (revision == lastRevision) return  // 幂等
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
        _tasks.value = newTasks
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
