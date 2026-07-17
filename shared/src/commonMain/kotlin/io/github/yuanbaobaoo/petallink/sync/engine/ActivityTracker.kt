package io.github.yuanbaobaoo.petallink.sync.engine

import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first

/**
 * 不可变活动状态
 */
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
    private val active = MutableStateFlow(0)

    /**
     * 判定两个路径是否重叠（相等或祖先/后代）
     */
    fun syncPathsOverlap(left: String, right: String): Boolean {
        if (left == right) return true
        if (left.startsWith(right) && left.removePrefix(right).startsWith("/")) return true
        if (right.startsWith(left) && right.removePrefix(left).startsWith("/")) return true
        return false
    }

    /**
     * 获取共享租约
     */
    fun begin(relativePath: String?): ActivityGuard? {
        while (true) {
            val cur = state.get()
            if (!cur.accepting) return null
            if (relativePath != null && cur.exclusivePaths.any { syncPathsOverlap(relativePath, it) }) return null
            val newActive = if (relativePath != null) {
                cur.activePaths + (relativePath to (cur.activePaths[relativePath] ?: 0) + 1)
            } else cur.activePaths
            val next = cur.copy(count = cur.count + 1, activePaths = newActive)
            if (state.compareAndSet(cur, next)) {
                active.value = next.count
                return ActivityGuard(Shared(relativePath), this)
            }
        }
    }

    /**
     * 获取独占租约
     */
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
            if (state.compareAndSet(cur, next)) {
                active.value = next.count
                return ActivityGuard(Exclusive(relativePath), this)
            }
        }
    }

    /**
     * 关闭追踪器
     */
    fun close() {
        while (true) {
            val cur = state.get()
            if (state.compareAndSet(cur, cur.copy(accepting = false))) return
        }
    }

    /**
     * 先封门，再等待封门前已登记的所有动作结算。
     */
    suspend fun closeAndWait() {
        close()
        active.first { it == 0 }
    }

    /**
     * 挂起直到所有租约释放、活动计数归零
     */
    suspend fun waitUntilIdle() {
        active.first { it == 0 }
    }

    /**
     * 释放租约
     */
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
            if (state.compareAndSet(cur, next)) {
                active.value = next.count
                return
            }
        }
    }

    /**
     * 当前持有的活动租约数量
     */
    fun activeCount(): Int = state.get().count
}

/**
 * 活动租约类型基类：区分共享与独占两种租约
 */
sealed class ActivityKind

/**
 * 共享活动租约：同路径可并发持有多个（path 为 null 表示不绑定路径）
 */
data class Shared(val path: String?) : ActivityKind()

/**
 * 独占活动租约：与同路径及其祖先/后代路径互斥
 */
data class Exclusive(val path: String) : ActivityKind()

/**
 * 活动租约守卫（RAII）：持有一种租约，[close] 时向 [tracker] 释放，保证只释放一次。
 */
class ActivityGuard(private val kind: ActivityKind, private val tracker: ActivityTracker) : AutoCloseable {
    private val closed = AtomicBoolean(false)

    /**
     * 释放持有的活动租约，保证只释放一次
     */
    override fun close() {
        if (closed.compareAndSet(false, true)) tracker.release(kind)
    }
}
