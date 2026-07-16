package io.github.yuanbaobaao.petallink

import io.github.yuanbaobaao.petallink.sync.*
import io.github.yuanbaobaao.petallink.sync.engine.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * 集成测试：完整同步流程（全 mock 数据）。
 * 测试 Planner.decide → AntiOscillation.filter → 结果聚合 的端到端路径。
 */
class IntegrationTest {

    @Test
    fun 全缺席路径无动作() {
        val snapshot = SyncSnapshot(
            local = emptyMap(), cloud = emptyMap(), db = emptyMap(),
            cloudTreeTrusted = true, isStartupResume = false,
        )
        val actions = Planner.plan(snapshot)
        assertEquals(0, actions.size)
    }

    @Test
    fun 新本地文件产生UPLOAD动作() {
        val local = mapOf("doc.txt" to LocalEntry("doc.txt", 100, 50, false, false))
        val snapshot = SyncSnapshot(local, emptyMap(), emptyMap(), true, false)
        val actions = Planner.plan(snapshot)
        assertEquals(1, actions.size)
        assertEquals(SyncActionType.UPLOAD, actions[0].type)
    }

    @Test
    fun 防振荡过滤丢弃最近删除路径的上传() {
        val antiOsc = AntiOscillation()
        antiOsc.addDeleted("doc.txt", 1000)
        val action = SyncAction(SyncActionType.UPLOAD, "doc.txt", null, null, "test")
        val result = antiOsc.filter(listOf(action))
        assertEquals(0, result.size)
    }

    @Test
    fun 防振荡保留DeleteFromCloud() {
        val antiOsc = AntiOscillation()
        antiOsc.addDeleted("doc.txt", 1000)
        val action = SyncAction(SyncActionType.DELETE_FROM_CLOUD, "doc.txt", "fid", null, "test")
        val result = antiOsc.filter(listOf(action))
        assertEquals(1, result.size)
    }

    @Test
    fun Reconciliation_rekeySubtree正确重映射路径() {
        val tree = mutableMapOf(
            "dir/file.txt" to io.github.yuanbaobaao.petallink.drive.DriveFile(id = "fid1", name = "file.txt"),
            "dir/sub/inner.txt" to io.github.yuanbaobaao.petallink.drive.DriveFile(id = "fid2", name = "inner.txt"),
        )
        val pathToId = mutableMapOf("dir/file.txt" to "fid1", "dir/sub/inner.txt" to "fid2")
        val idToPath = Reconciliation.buildIdToPath(pathToId).toMutableMap()

        Reconciliation.rekeySubtree(tree, pathToId, idToPath, "dir", "renamed")

        // 验证重映射结果
        assertTrue(tree.containsKey("renamed/file.txt"))
        assertTrue(tree.containsKey("renamed/sub/inner.txt"))
        assertEquals("fid1", pathToId["renamed/file.txt"])
        assertEquals("fid2", pathToId["renamed/sub/inner.txt"])
    }
}
