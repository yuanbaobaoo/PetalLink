package io.github.yuanbaobaao.petallink.core.net_guard

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/**
 * JVM 网络守卫实现（actual）。
 * TCP 连接探测 driveapis.cloud.huawei.com.cn:443，3s 超时，连续 2 次成功才 ONLINE。
 */
actual class NetGuard actual constructor() {
    private val engine = NetGuardEngine()
    private val generation = AtomicInteger(0)
    private val scopeRef = AtomicReference<CoroutineScope?>(null)

    actual val state: NetState get() = engine.state()

    actual fun startProbe() {
        if (scopeRef.get() != null) return
        val s = CoroutineScope(Dispatchers.IO)
        scopeRef.set(s)
        s.launch {
            while (true) {
                val gen = generation.get()
                val success = probeOnce()
                engine.onProbeResult(success, gen)
                delay(PROBE_INTERVAL_MS)
            }
        }
    }

    actual fun stopProbe() {
        scopeRef.get()?.cancel()
        scopeRef.set(null)
    }

    actual fun newGeneration(): Int {
        generation.incrementAndGet()
        return generation.get()
    }

    private suspend fun probeOnce(): Boolean {
        return withTimeoutOrNull(PROBE_TIMEOUT_MS) {
            try {
                Socket().use { socket ->
                    socket.connect(InetSocketAddress(PROBE_HOST, PROBE_PORT), PROBE_TIMEOUT_MS.toInt())
                    socket.isConnected
                }
            } catch (e: Throwable) {
                false
            }
        } ?: false
    }

    private companion object {
        const val PROBE_HOST = "driveapis.cloud.huawei.com.cn"
        const val PROBE_PORT = 443
        const val PROBE_TIMEOUT_MS = 3_000L
        const val PROBE_INTERVAL_MS = 30_000L
    }
}
