package io.github.yuanbaobaao.petallink.update

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlinx.coroutines.test.runTest

class UpdateModelsTest {
    @Test
    fun 语义版本只接受更高版本() {
        assertTrue(UpdateManifest("1.2.0", "https://example.test/app.zip", "a".repeat(64)).isNewerThan("1.1.9"))
        assertFalse(UpdateManifest("1.1.9", "https://example.test/app.zip", "a".repeat(64)).isNewerThan("1.2.0"))
        assertFalse(UpdateManifest("invalid", "https://example.test/app.zip", "a".repeat(64)).isNewerThan("1.2.0"))
    }

    @Test
    fun 更新最多等待传输五分钟() = runTest {
        var now = 0L
        val waiter = TransferIdleWaiter({ true }, { now }) { now += it }
        assertFalse(waiter.await())
        assertTrue(now >= TransferIdleWaiter.MAX_WAIT_MS)
    }

    @Test
    fun 传输结束后立即放行更新() = runTest {
        var active = true
        var now = 0L
        val waiter = TransferIdleWaiter({ active }, { now }) { now += it; active = false }
        assertTrue(waiter.await())
    }
}
