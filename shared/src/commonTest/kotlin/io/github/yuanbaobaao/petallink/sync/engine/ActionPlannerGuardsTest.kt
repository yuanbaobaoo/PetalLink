package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.drive.DriveFile
import io.github.yuanbaobaao.petallink.sync.DbBaselineEntry
import io.github.yuanbaobaao.petallink.sync.LocalEntry
import io.github.yuanbaobaao.petallink.sync.SyncAction
import io.github.yuanbaobaao.petallink.sync.SyncActionType
import io.github.yuanbaobaao.petallink.sync.SyncSnapshot
import io.github.yuanbaobaao.petallink.sync.SyncStatus
import io.github.yuanbaobaao.petallink.sync.executor.ActionResult
import io.github.yuanbaobaao.petallink.sync.executor.SyncExecutor
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse

class ActionPlannerGuardsTest {
    @Test
    fun 子项需备份时保留本地父目录() {
        val snapshot = snapshot(local = mapOf(
            "dir" to local("dir", folder = true),
            "dir/a" to local("dir/a"),
        ))
        val actions = listOf(
            action(SyncActionType.DELETE_FROM_LOCAL, "dir"),
            action(SyncActionType.BACKUP_BEFORE_CLOUD_DELETE, "dir/a"),
        )
        val result = ActionPlannerGuards.prepare(snapshot, actions)
        assertFalse(result.any { it.type == SyncActionType.DELETE_FROM_LOCAL && it.relativePath == "dir" })
    }

    @Test
    fun 被云端删除的祖先为救援上传补CreateFolder() {
        val snapshot = snapshot(
            local = mapOf("dir" to local("dir", true), "dir/a" to local("dir/a")),
            db = mapOf("dir" to db("folder")),
        )
        val result = ActionPlannerGuards.prepare(snapshot, listOf(action(SyncActionType.UPLOAD, "dir/a")))
        assertEquals(SyncActionType.CREATE_FOLDER, result.first { it.relativePath == "dir" }.type)
    }

    @Test
    fun 两阶段目录创建回填parent且结果严格对应原索引() = runBlocking {
        val executed = mutableListOf<SyncAction>()
        val executor = SyncExecutor { action ->
            executed += action
            val id = when (action.relativePath) { "a" -> "id-a"; "a/b" -> "id-b"; else -> null }
            ActionResult(success = true, cloudFileId = id)
        }
        val actions = listOf(
            action(SyncActionType.UPLOAD, "a/b/file.txt"),
            action(SyncActionType.CREATE_FOLDER, "a/b"),
            action(SyncActionType.CREATE_FOLDER, "a"),
        )
        val results = executor.executeActionsOrdered(actions, mapOf("" to "root"))

        assertEquals(listOf("a", "a/b", "a/b/file.txt"), executed.map { it.relativePath })
        assertEquals("root", executed[0].parentFileId)
        assertEquals("id-a", executed[1].parentFileId)
        assertEquals("id-b", executed[2].parentFileId)
        assertEquals(3, results.size)
        assertEquals("id-a", results[2].cloudFileId)
        assertEquals("id-b", results[1].cloudFileId)
    }

    @Test
    fun 父目录创建失败时子目录和上传不得落到根目录() = runBlocking {
        val executed = mutableListOf<String>()
        val executor = SyncExecutor { action ->
            executed += action.relativePath
            ActionResult(success = action.relativePath != "a")
        }
        val results = executor.executeActionsOrdered(
            listOf(action(SyncActionType.CREATE_FOLDER, "a"), action(SyncActionType.UPLOAD, "a/file")),
            mapOf("" to "root"),
        )
        assertEquals(listOf("a"), executed)
        assertFalse(results[1].success)
    }

    private fun action(type: SyncActionType, path: String) = SyncAction(type, path, reason = "test")
    private fun local(path: String, folder: Boolean = false) = LocalEntry(path, 1, 1, false, folder)
    private fun db(id: String) = DbBaselineEntry(id, 1, 1, 1, SyncStatus.SYNCED, true)
    private fun snapshot(
        local: Map<String, LocalEntry> = emptyMap(),
        cloud: Map<String, DriveFile> = emptyMap(),
        db: Map<String, DbBaselineEntry> = emptyMap(),
    ) = SyncSnapshot(local, cloud, db, cloudTreeTrusted = true, isStartupResume = false)
}
