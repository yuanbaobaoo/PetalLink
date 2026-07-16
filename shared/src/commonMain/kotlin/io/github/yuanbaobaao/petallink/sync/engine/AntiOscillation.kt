package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.config.AppConfig
import io.github.yuanbaobaao.petallink.sync.SyncAction
import io.github.yuanbaobaao.petallink.sync.SyncActionType
import java.util.concurrent.atomic.AtomicReference

/**
 * 防振荡逻辑（对标 src/sync/engine/results.rs + action_filters.rs）。
 *
 * recentlyDeletedPaths：本地删除后 5 分钟内丢弃同路径回弹动作（保留 DeleteFromCloud）。
 * 用 AtomicReference + CAS 无锁实现。
 */
class AntiOscillation {
    // relative_path → 删除时间戳（ms），不可变 Map 用于 CAS
    private val state = AtomicReference<Map<String, Long>>(emptyMap())

    /** 记录已删除路径 */
    fun addDeleted(relativePath: String, nowMs: Long) {
        while (true) {
            val cur = state.get()
            if (state.compareAndSet(cur, cur + (relativePath to nowMs))) return
        }
    }

    /** TTL 清理：移除超过 5 分钟的记录 */
    fun purgeExpired(nowMs: Long) {
        val expireBefore = nowMs - AppConfig.ANTI_OSCILLATION_TTL.inWholeMilliseconds
        while (true) {
            val cur = state.get()
            val next = cur.filter { it.value > expireBefore }
            if (state.compareAndSet(cur, next)) return
        }
    }

    /** 过滤振荡动作（保留 DeleteFromCloud） */
    fun filter(actions: List<SyncAction>): List<SyncAction> {
        val rdp = state.get()
        return actions.filter { action ->
            if (action.relativePath !in rdp) return@filter true
            action.type == SyncActionType.DELETE_FROM_CLOUD
        }
    }

    fun contains(relativePath: String): Boolean = relativePath in state.get()
    fun size(): Int = state.get().size
}
