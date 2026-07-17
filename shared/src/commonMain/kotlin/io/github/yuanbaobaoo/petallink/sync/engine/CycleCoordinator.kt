package io.github.yuanbaobaoo.petallink.sync.engine

import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex

/**
 * 同步周期请求位集（对标 src/sync/engine/coordination.rs CycleRequest）。
 *
 * 7 位位集，每位代表一种触发来源。详见 docs/06 §8。
 */
data class CycleRequest(val bits: Int) {
    fun contains(other: CycleRequest): Boolean = bits and other.bits == other.bits
    fun isEmpty(): Boolean = bits == 0
    operator fun plus(other: CycleRequest): CycleRequest = CycleRequest(bits or other.bits)

    companion object {
        val LOCAL_RESCAN = CycleRequest(1 shl 0)
        val CLOUD_INCREMENTAL = CycleRequest(1 shl 1)
        val CLOUD_FULL = CycleRequest(1 shl 2)
        val ONLINE_RECOVERY = CycleRequest(1 shl 3)
        val STARTUP = CycleRequest(1 shl 4)
        val RETRY = CycleRequest(1 shl 5)
        val REPLAN = CycleRequest(1 shl 6)
        val EMPTY = CycleRequest(0)
    }
}

/** 触发来源 → CycleRequest 映射（对标 cycle_request_for_trigger） */
object CycleTrigger {
    const val MANUAL_REFRESH = "manual-refresh"
    const val AUTO_CLOUD_REFRESH = "auto-cloud-refresh"
    const val NETWORK_RECOVERY = "network-recovery"
    const val STARTUP_RESUME = "startup-resume"
    const val RETRY_FAILED = "retry-failed"
    const val RETRY_REPLAN = "retry-replan"
    const val BACKOFF_DEADLINE = "backoff-deadline"

    fun requestFor(trigger: String): CycleRequest = when (trigger) {
        MANUAL_REFRESH -> CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_FULL
        AUTO_CLOUD_REFRESH -> CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL
        NETWORK_RECOVERY -> CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL + CycleRequest.ONLINE_RECOVERY
        STARTUP_RESUME -> CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL + CycleRequest.ONLINE_RECOVERY + CycleRequest.STARTUP
        RETRY_FAILED -> CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL + CycleRequest.RETRY
        RETRY_REPLAN -> CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL + CycleRequest.REPLAN
        BACKOFF_DEADLINE -> CycleRequest.LOCAL_RESCAN + CycleRequest.CLOUD_INCREMENTAL + CycleRequest.ONLINE_RECOVERY
        else -> CycleRequest.LOCAL_RESCAN
    }
}

/** 不可变协调器状态（用于 CAS） */
private data class CoordinatorState(
    val pending: Int = 0,
    val requestedSeq: Long = 0,
    val completedSeq: Long = 0,
    val failures: List<CycleFailure> = emptyList(),
)

private data class CycleFailure(val start: Long, val end: Long, val message: String)

/**
 * 同步周期协调器（对标 CycleCoordinator）。
 * 用 AtomicReference + CAS 循环实现无锁线程安全。
 */
class CycleCoordinator {
    private val state = AtomicReference(CoordinatorState())
    private val owner = Mutex()

    /** 合并请求到位集，返回请求序号 */
    fun request(req: CycleRequest): Long {
        while (true) {
            val cur = state.get()
            val newSeq = if (cur.requestedSeq == Long.MAX_VALUE) 1 else cur.requestedSeq + 1
            val next = cur.copy(pending = cur.pending or req.bits, requestedSeq = newSeq)
            if (state.compareAndSet(cur, next)) return newSeq
        }
    }

    /** 取出 pending 位集（清空），返回 (位集, 序号) */
    fun takePending(): Pair<CycleRequest, Long> {
        while (true) {
            val cur = state.get()
            val next = cur.copy(pending = 0)
            if (state.compareAndSet(cur, next)) return CycleRequest(cur.pending) to cur.requestedSeq
        }
    }

    /** 恢复请求（sticky） */
    fun restore(request: CycleRequest) {
        while (true) {
            val cur = state.get()
            val next = cur.copy(pending = cur.pending or request.bits)
            if (state.compareAndSet(cur, next)) return
        }
    }

    /** 完成一个请求序号 */
    fun complete(through: Long, error: String? = null) {
        while (true) {
            val cur = state.get()
            val newFailures = if (error != null) {
                val list = cur.failures + CycleFailure(cur.completedSeq + 1, through, error)
                if (list.size > 128) list.takeLast(128) else list
            } else cur.failures
            val next = cur.copy(
                completedSeq = maxOf(cur.completedSeq, through),
                failures = newFailures,
            )
            if (state.compareAndSet(cur, next)) return
        }
    }

    /** 查询某序号的结果 */
    fun resultIfCompleted(seq: Long): Result<Unit>? {
        val cur = state.get()
        if (seq > cur.completedSeq) return null
        for (failure in cur.failures) {
            if (seq in failure.start..failure.end) return Result.failure(RuntimeException(failure.message))
        }
        return Result.success(Unit)
    }

    /** 是否有未完成的请求 */
    fun hasUncompletedRequest(): Boolean = state.get().requestedSeq > state.get().completedSeq

    fun hasPending(): Boolean = state.get().pending != 0

    /** 同一时刻仅一个 owner 可以取走并执行合并后的周期请求。 */
    suspend fun drainOwned(execute: suspend (CycleRequest) -> Result<Unit>): Boolean {
        if (!owner.tryLock()) return false
        try {
            while (true) {
                val (request, sequence) = takePending()
                if (request.isEmpty()) break
                val result = runCatching { execute(request) }.getOrElse { Result.failure(it) }
                complete(sequence, result.exceptionOrNull()?.message)
            }
            return true
        } finally {
            owner.unlock()
        }
    }
}

/** watcher/manual/timer/startup/recovery 统一只向这个入口提交请求。 */
class CycleRequestDispatcher(
    private val scope: CoroutineScope,
    private val coordinator: CycleCoordinator,
    private val execute: suspend (CycleRequest) -> Result<Unit>,
) {
    private val workerScheduled = AtomicBoolean(false)

    fun submit(request: CycleRequest): Long {
        val sequence = coordinator.request(request)
        schedule()
        return sequence
    }

    fun submit(trigger: String): Long = submit(CycleTrigger.requestFor(trigger))

    private fun schedule() {
        if (!workerScheduled.compareAndSet(false, true)) return
        scope.launch {
            try {
                coordinator.drainOwned(execute)
            } finally {
                workerScheduled.set(false)
                if (coordinator.hasPending()) schedule()
            }
        }
    }
}
