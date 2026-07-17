package io.github.yuanbaobaoo.petallink.core.net_guard

import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * 使用应用级 CoroutineScope 的单实例网络守卫。
 */
actual class NetGuard actual constructor(private val scope: CoroutineScope) {
    private val engine = NetGuardEngine()
    private val generation = AtomicInteger(0)
    private val mutableState = MutableStateFlow(NetState.OFFLINE)
    private val lock = Any()
    private var probeJob: Job? = null

    actual val state: StateFlow<NetState> = mutableState.asStateFlow()

    actual fun startProbe() = synchronized(lock) {
        if (probeJob?.isActive == true) return@synchronized
        val gen = newGeneration()
        probeJob = scope.launch {
            while (isActive && generation.get() == gen) {
                publishProbeResult(probeOnce(), gen)
                delay(PROBE_INTERVAL_MS)
            }
        }
    }

    actual fun stopProbe() = synchronized(lock) {
        generation.incrementAndGet() // 先失效当前代际，再取消任务。
        probeJob?.cancel()
        probeJob = null
    }

    actual fun newGeneration(): Int = generation.updateAndGet { current ->
        if (current == Int.MAX_VALUE) 1 else current + 1
    }

    actual fun reportRequestNetworkFailure(): Boolean = synchronized(engine) {
        val changed = engine.onRequestNetworkFailure()
        mutableState.value = engine.state()
        changed
    }

    /**
     * 仅当当前代际匹配时，将单次探测结果反馈给引擎并更新对外状态。
     */
    private fun publishProbeResult(success: Boolean, gen: Int) = synchronized(engine) {
        if (gen != generation.get()) return@synchronized
        mutableState.value = engine.onProbeResult(success, gen)
    }

    /**
     * 对华为域名 443 端口执行一次带超时的 TCP 连接探测，成功返回 true。
     */
    private suspend fun probeOnce(): Boolean = withTimeoutOrNull(PROBE_TIMEOUT_MS) {
        try {
            Socket().use { socket ->
                socket.connect(InetSocketAddress(PROBE_HOST, PROBE_PORT), PROBE_TIMEOUT_MS.toInt())
                socket.isConnected
            }
        } catch (_: Throwable) {
            false
        }
    } ?: false

    private companion object {
        const val PROBE_HOST = "driveapis.cloud.huawei.com.cn"
        const val PROBE_PORT = 443
        const val PROBE_TIMEOUT_MS = 3_000L
        const val PROBE_INTERVAL_MS = 30_000L
    }
}
