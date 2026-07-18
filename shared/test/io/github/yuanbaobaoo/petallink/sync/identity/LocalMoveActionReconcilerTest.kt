package io.github.yuanbaobaoo.petallink.sync.identity

import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.sync.SyncAction
import io.github.yuanbaobaoo.petallink.sync.SyncActionType
import io.github.yuanbaobaoo.petallink.sync.engine.CloudTreeCache
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * 本地 inode 移动到云端动作的纯逻辑测试。
 */
class LocalMoveActionReconcilerTest {

    @Test
    fun 文件重命名替换删除和上传并保留云端身份() {
        val remote = DriveFile(id = "file-1", name = "old.txt", parentFolder = listOf("root"))
        val cloud = CloudTreeCache.trusted(
            tree = mapOf("old.txt" to remote),
            pathToId = mapOf("old.txt" to "file-1"),
            rootFolderId = "root",
            cursor = "cursor",
        )
        val planned = listOf(
            SyncAction(SyncActionType.DELETE_FROM_CLOUD, "old.txt", "file-1", remote, "delete"),
            SyncAction(SyncActionType.UPLOAD, "new.txt", reason = "upload"),
        )

        val result = LocalMoveActionReconciler.reconcile(
            planned,
            listOf(DetectedMove(7UL, "file-1", "old.txt", "new.txt")),
            cloud,
        )

        assertEquals(1, result.size)
        assertEquals(SyncActionType.MOVE_IN_CLOUD, result.single().type)
        assertEquals("file-1", result.single().fileId)
        assertEquals("root", result.single().parentFileId)
    }

    @Test
    fun 目录移动折叠全部后代动作() {
        val moves = listOf(
            DetectedMove(1UL, "dir", "old", "new"),
            DetectedMove(2UL, "child", "old/a.txt", "new/a.txt"),
        )

        assertEquals(listOf(moves.first()), LocalMoveActionReconciler.collapseNested(moves))
    }
}
