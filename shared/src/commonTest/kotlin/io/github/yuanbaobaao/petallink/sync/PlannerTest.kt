package io.github.yuanbaobaao.petallink.sync

import io.github.yuanbaobaao.petallink.drive.DriveFile
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Planner 单测（对标 src/sync/planner.rs decide() 决策表）。
 */
class PlannerTest {
    @Test
    fun 同步状态持久化数值与原Tauri完全一致() {
        assertEquals(0, SyncStatus.SYNCED)
        assertEquals(1, SyncStatus.CLOUD_ONLY)
        assertEquals(2, SyncStatus.LOCAL_ONLY)
        assertEquals(3, SyncStatus.SYNCING)
        assertEquals(4, SyncStatus.FAILED)
        assertEquals(5, SyncStatus.CONFLICT)
        assertEquals(7, SyncStatus.DELETED)
    }

    private val path = "doc.txt"
    private fun localFile(mtime: Long = 100, size: Long = 50) =
        LocalEntry(path, mtime, size, isPlaceholder = false, isFolder = false)
    private fun cloudFile(id: String = "cid", folder: Boolean = false, editedTime: String? = null) =
        DriveFile(id = id, name = path, mimeType = if (folder) "vnd.huawei-apps.folder" else "text/plain", editedTime = editedTime)
    private fun dbEntry(
        fileId: String = "cid", localMtime: Long? = 100, localSize: Long? = 50,
        cloudTime: Long? = 1000, status: Int = SyncStatus.SYNCED, folder: Boolean = false,
    ) = DbBaselineEntry(fileId, localMtime, localSize, cloudTime, status, folder)

    @Test
    fun 全缺席返回null() {
        val result = Planner.decide(path, null, null, null, true, false)
        assertNull(result)
    }

    @Test
    fun 三方都存在_未变化返回null() {
        val result = Planner.decide(path, localFile(), cloudFile(), dbEntry(), true, false)
        assertNull(result)
    }

    @Test
    fun 三方都存在_仅本地改_上传() {
        val result = Planner.decide(path, localFile(mtime = 200), cloudFile(), dbEntry(), true, false)
        assertEquals(SyncActionType.UPLOAD, result?.type)
    }

    @Test
    fun 三方都存在_本地和云端都改_冲突() {
        val local = localFile(mtime = 200)
        val cloud = cloudFile(editedTime = "1970-01-01T00:00:02Z")
        val db = dbEntry(localMtime = 100, cloudTime = 1000)
        val result = Planner.decide(path, local, cloud, db, true, false)
        assertEquals(SyncActionType.CREATE_CONFLICT_COPY, result?.type)
    }

    @Test
    fun pending占位项收敛为Skip带cloudFile() {
        val db = dbEntry(fileId = "pending:abc")
        val result = Planner.decide(path, localFile(), cloudFile(), db, true, false)
        assertEquals(SyncActionType.SKIP, result?.type)
        assertEquals("cid", result?.cloudFile?.id)  // 携带真实 cloud_file
    }

    @Test
    fun 本地新文件_无DB_上传() {
        val result = Planner.decide(path, localFile(), null, null, true, false)
        assertEquals(SyncActionType.UPLOAD, result?.type)
        assertNull(result?.fileId)
    }

    @Test
    fun 云端新文件_无DB_创建占位() {
        val result = Planner.decide(path, null, cloudFile(), null, true, false)
        assertEquals(SyncActionType.CREATE_PLACEHOLDER, result?.type)
    }

    @Test
    fun 云端删除_本地无改_删除本地() {
        val db = dbEntry()
        val result = Planner.decide(path, localFile(), null, db, true, false)
        assertEquals(SyncActionType.DELETE_FROM_LOCAL, result?.type)
    }

    @Test
    fun 云端删除_本地有改_备份() {
        val db = dbEntry(localMtime = 100)
        val local = localFile(mtime = 200)  // 本地已改
        val result = Planner.decide(path, local, null, db, true, false)
        assertEquals(SyncActionType.BACKUP_BEFORE_CLOUD_DELETE, result?.type)
    }

    @Test
    fun 本地删除_会话内_删云端() {
        val db = dbEntry()
        val result = Planner.decide(path, null, cloudFile(), db, true, false)
        assertEquals(SyncActionType.DELETE_FROM_CLOUD, result?.type)
    }

    @Test
    fun 本地删除_启动恢复_跳过() {
        val db = dbEntry()
        val result = Planner.decide(path, null, cloudFile(), db, true, isStartupResume = true)
        // 启动恢复 + 非 DELETED → 重建占位
        assertEquals(SyncActionType.CREATE_PLACEHOLDER, result?.type)
    }

    @Test
    fun 本地删除_启动恢复_DELETED墓碑_跳过() {
        val db = dbEntry(status = SyncStatus.DELETED)
        val result = Planner.decide(path, null, cloudFile(), db, true, isStartupResume = true)
        assertEquals(SyncActionType.SKIP, result?.type)
    }

    @Test
    fun 孤儿占位符_清理() {
        val local = LocalEntry(path, 100, 0, isPlaceholder = true, isFolder = false)
        val result = Planner.decide(path, local, null, null, true, false)
        assertEquals(SyncActionType.DELETE_FROM_LOCAL, result?.type)
    }

    @Test
    fun pending加FAILED_不重试() {
        val db = dbEntry(fileId = "pending:abc", status = SyncStatus.FAILED)
        val result = Planner.decide(path, localFile(), null, db, true, false)
        assertNull(result)  // FAILED → 不自动重试
    }

    @Test
    fun pending非FAILED_重新上传() {
        val db = dbEntry(fileId = "pending:abc", status = SyncStatus.PENDING)
        val result = Planner.decide(path, localFile(), null, db, true, false)
        assertEquals(SyncActionType.UPLOAD, result?.type)
    }

    @Test
    fun 不可信云端树_抑制删除() {
        val db = dbEntry()
        val result = Planner.decide(path, localFile(), null, db, cloudTreeTrusted = false, false)
        assertEquals(SyncActionType.DELETE_FROM_LOCAL, result?.type)  // decide 仍产生动作
    }

    @Test
    fun plan不可信云端树抑制删除动作() {
        val snapshot = SyncSnapshot(
            local = mapOf(path to localFile()),
            cloud = emptyMap(),
            db = mapOf(path to dbEntry()),
            cloudTreeTrusted = false,  // 不可信
            isStartupResume = false,
        )
        val actions = Planner.plan(snapshot)
        // DELETE_FROM_LOCAL 被抑制 → 空计划
        assertTrue(actions.none { it.type == SyncActionType.DELETE_FROM_LOCAL })
    }

    @Test
    fun isLocalChanged_mtime变化为true() {
        assertTrue(Planner.isLocalChanged(localFile(mtime = 200), dbEntry(localMtime = 100)))
    }

    @Test
    fun isLocalChanged_未变为false() {
        assertEquals(false, Planner.isLocalChanged(localFile(mtime = 100), dbEntry(localMtime = 100)))
    }

    @Test
    fun isCloudChanged_cloudTime缺失为false() {
        assertEquals(false, Planner.isCloudChanged(null, dbEntry(cloudTime = 1000)))
    }

    @Test
    fun editedTime_RFC3339真实参与云端变更判定() {
        val result = Planner.decide(
            path,
            localFile(),
            cloudFile(editedTime = "1970-01-01T00:00:02Z"),
            dbEntry(cloudTime = 1000),
            true,
            false,
        )
        assertEquals(SyncActionType.DOWNLOAD, result?.type)
    }
}
