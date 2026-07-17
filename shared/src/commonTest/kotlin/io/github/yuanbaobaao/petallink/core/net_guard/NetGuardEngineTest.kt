package io.github.yuanbaobaao.petallink.core.net_guard

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * NetGuardEngine 纯逻辑单测（对标 src/core/net_guard.rs 防抖）。
 */
class NetGuardEngineTest {

    @Test
    fun 失败立即转OFFLINE() {
        val engine = NetGuardEngine()
        assertEquals(NetState.OFFLINE, engine.onProbeResult(false, gen = 1))
    }

    @Test
    fun 首次成功不转ONLINE_需连续2次() {
        val engine = NetGuardEngine()
        // 第 1 次成功仍 OFFLINE（防抖：需连续 2 次）
        assertEquals(NetState.OFFLINE, engine.onProbeResult(true, gen = 1))
        // 第 2 次成功转 ONLINE
        assertEquals(NetState.ONLINE, engine.onProbeResult(true, gen = 1))
    }

    @Test
    fun 成功与失败交替不满足防抖() {
        val engine = NetGuardEngine()
        engine.onProbeResult(true, gen = 1)   // 连续计数=1
        engine.onProbeResult(false, gen = 1)  // 失败立即清零并 OFFLINE
        // 再次第 1 次成功仍 OFFLINE（连续计数被重置）
        assertEquals(NetState.OFFLINE, engine.onProbeResult(true, gen = 1))
    }

    @Test
    fun ONLINE后一次失败立即转OFFLINE() {
        val engine = NetGuardEngine()
        engine.onProbeResult(true, gen = 1)
        engine.onProbeResult(true, gen = 1)   // 转 ONLINE
        assertEquals(NetState.OFFLINE, engine.onProbeResult(false, gen = 1))
    }

    @Test
    fun 代际切换重置连续计数() {
        val engine = NetGuardEngine()
        engine.onProbeResult(true, gen = 1)   // gen=1, 连续=1
        // gen=2 到来，重置连续=1（新代际首次成功）
        assertEquals(NetState.OFFLINE, engine.onProbeResult(true, gen = 2))
        // gen=2 第 2 次成功才转 ONLINE
        assertEquals(NetState.ONLINE, engine.onProbeResult(true, gen = 2))
    }

    @Test
    fun 旧代际回调被忽略() {
        val engine = NetGuardEngine()
        engine.onProbeResult(true, gen = 1)   // gen=1, 连续=1
        engine.onProbeResult(true, gen = 2)   // 新代际，重置连续=1
        // gen=1 的迟到成功回调：代际切回 1，重置连续=1，仍 OFFLINE
        assertEquals(NetState.OFFLINE, engine.onProbeResult(true, gen = 1))
    }

    @Test
    fun 被动失败只发布一次离线边沿() {
        val engine = NetGuardEngine()
        engine.onProbeResult(true, gen = 1)
        engine.onProbeResult(true, gen = 1)
        assertTrue(engine.onRequestNetworkFailure())
        assertFalse(engine.onRequestNetworkFailure())
    }
}
