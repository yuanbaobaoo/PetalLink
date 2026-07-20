package io.github.yuanbaobaoo.petallink.core.net_guard

import io.github.yuanbaobaoo.petallink.core.logging.Logger
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
    private val logger = Logger()
    private val engine = NetGuardEngine()
    private val generation = AtomicInteger(0)
    private val mutableState = MutableStateFlow(NetState.OFFLINE)
    private val lock = Any()
    private var probeJob: Job? = null

    actual val state: StateFlow<NetState> = mutableState.asStateFlow()

    actual fun startProbe() = synchronized(lock) {
        if (probeJob?.isActive == true) return@synchronized
        val gen = newGeneration()
        logger.info("core.net_guard") { "网络探测任务已启动（间隔 ${PROBE_INTERVAL_MS / 1000}s）" }
        probeJob = scope.launch {
            try {
                while (isActive && generation.get() == gen) {
                    publishProbeResult(probeOnce(), gen)
                    delay(PROBE_INTERVAL_MS)
                }
            } finally {
                logger.info("core.net_guard") { "网络探测任务检测到 shutdown，退出循环" }
            }
        }
    }

    actual fun stopProbe() = synchronized(lock) {
        logger.info("core.net_guard") { "网络探测任务已请求停止" }
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
        val previous = mutableState.value
        val current = engine.onProbeResult(success, gen)
        mutableState.value = current
        if (previous != current) {
            if (current == NetState.ONLINE) {
                logger.info("core.net_guard") { "网络状态：在线（恢复同步）" }
            } else {
                logger.warn("core.net_guard") { "网络状态：离线（探测失败，暂停同步）" }
            }
        }
    }

    /**
     * 对华为域名 443 端口执行一次带超时的 TCP 连接探测，成功返回 true。
     */
    private suspend fun probeOnce(): Boolean {
        val result = withTimeoutOrNull(PROBE_TIMEOUT_MS) {
            try {
                Socket().use { socket ->
                    socket.connect(InetSocketAddress(PROBE_HOST, PROBE_PORT), PROBE_TIMEOUT_MS.toInt())
                    socket.isConnected
                }
            } catch (e: Throwable) {
                logger.debug("core.net_guard") { "网络探测连接失败：${e.message}" }
                false
            }
        }
        if (result == null) logger.debug("core.net_guard") { "网络探测超时（${PROBE_TIMEOUT_MS / 1000}s）" }
        return result ?: false
    }

    private companion object {
        const val PROBE_HOST = "driveapis.cloud.huawei.com.cn"
        const val PROBE_PORT = 443
        const val PROBE_TIMEOUT_MS = 3_000L
        const val PROBE_INTERVAL_MS = 30_000L
    }
}
