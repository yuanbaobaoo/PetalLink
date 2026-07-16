package io.github.yuanbaobaao.petallink.sync.engine

import java.util.concurrent.atomic.AtomicReference

/** 不可变活动状态 */
private data class ActivityState(
    val accepting: Boolean = true,
    val count: Int = 0,
    val activePaths: Map<String, Int> = emptyMap(),
    val exclusivePaths: Set<String> = emptySet(),
)

/**
 * 活动追踪器（对标 src/sync/engine/coordination.rs ActivityTracker）。
 *
 * 路径租约：共享（同路径可多个）/ 独占（同路径或祖先/后代互斥）。
 * 用 AtomicReference + CAS 无锁实现。
 */
class ActivityTracker {
    private val state = AtomicReference(ActivityState())

    /** 判定两个路径是否重叠（相等或祖先/后代） */
    fun syncPathsOverlap(left: String, right: String): Boolean {
        if (left == right) return true
        if (left.startsWith(right) && left.removePrefix(right).startsWith("/")) return true
        if (right.startsWith(left) && right.removePrefix(left).startsWith("/")) return true
        return false
    }

    /** 获取共享租约 */
    fun begin(relativePath: String?): ActivityGuard? {
        while (true) {
            val cur = state.get()
            if (!cur.accepting) return null
            if (relativePath != null && cur.exclusivePaths.any { syncPathsOverlap(relativePath, it) }) return null
            val newActive = if (relativePath != null) {
                cur.activePaths + (relativePath to (cur.activePaths[relativePath] ?: 0) + 1)
            } else cur.activePaths
            val next = cur.copy(count = cur.count + 1, activePaths = newActive)
            if (state.compareAndSet(cur, next)) return ActivityGuard(Shared(relativePath), this)
        }
    }

    /** 获取独占租约 */
    fun beginExclusive(relativePath: String): ActivityGuard? {
        while (true) {
            val cur = state.get()
            if (!cur.accepting) return null
            if (cur.activePaths.keys.any { syncPathsOverlap(relativePath, it) }) return null
            if (cur.exclusivePaths.any { syncPathsOverlap(relativePath, it) }) return null
            val next = cur.copy(
                count = cur.count + 1,
                exclusivePaths = cur.exclusivePaths + relativePath,
            )
            if (state.compareAndSet(cur, next)) return ActivityGuard(Exclusive(relativePath), this)
        }
    }

    /** 关闭追踪器 */
    fun close() {
        while (true) {
            val cur = state.get()
            if (state.compareAndSet(cur, cur.copy(accepting = false))) return
        }
    }

    /** 释放租约 */
    fun release(kind: ActivityKind) {
        while (true) {
            val cur = state.get()
            val next = when (kind) {
                is Shared -> {
                    val path = kind.path
                    val newActive = if (path != null) {
                        val c = (cur.activePaths[path] ?: 0) - 1
                        if (c <= 0) cur.activePaths - path else cur.activePaths + (path to c)
                    } else cur.activePaths
                    cur.copy(count = cur.count - 1, activePaths = newActive)
                }
                is Exclusive -> cur.copy(
                    count = cur.count - 1,
                    exclusivePaths = cur.exclusivePaths - kind.path,
                )
            }
            if (state.compareAndSet(cur, next)) return
        }
    }

    fun activeCount(): Int = state.get().count
}

sealed class ActivityKind
data class Shared(val path: String?) : ActivityKind()
data class Exclusive(val path: String) : ActivityKind()

class ActivityGuard(private val kind: ActivityKind, private val tracker: ActivityTracker) {
    fun close() = tracker.release(kind)
}
