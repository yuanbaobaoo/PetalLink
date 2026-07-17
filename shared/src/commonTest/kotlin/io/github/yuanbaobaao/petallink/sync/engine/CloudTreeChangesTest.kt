package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.drive.ChangeKind
import io.github.yuanbaobaao.petallink.drive.DriveChange
import io.github.yuanbaobaao.petallink.drive.DriveFile
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class CloudTreeChangesTest {
    @Test
    fun 目录改名会重键整个子树() {
        val cache = cache()
        val renamed = DriveFile(id = "folder", name = "renamed", parent = "root", mimeType = "application/vnd.huawei-apps.folder")

        val result = CloudTreeChanges.apply(
            cache,
            listOf(DriveChange(ChangeKind.MODIFIED, "folder", renamed)),
            "c2",
        )

        assertFalse("docs" in result.tree)
        assertFalse("docs/a.txt" in result.tree)
        assertEquals("folder", result.pathToId["renamed"])
        assertEquals("child", result.pathToId["renamed/a.txt"])
        assertTrue(result.isTrusted())
    }

    @Test
    fun 删除目录会移除整个子树() {
        val result = CloudTreeChanges.apply(
            cache(),
            listOf(DriveChange(ChangeKind.REMOVED, "folder", null)),
            "c2",
        )
        assertTrue(result.tree.isEmpty())
        assertEquals(mapOf("" to "root"), result.pathToId)
    }

    @Test
    fun 不可解析父目录和路径冲突必须整批失败() {
        val orphan = DriveFile(id = "new", name = "new.txt", parent = "missing")
        assertFailsWith<AppError.Data> {
            CloudTreeChanges.apply(cache(), listOf(DriveChange(ChangeKind.MODIFIED, "new", orphan)), "c2")
        }
        assertEquals(setOf("docs", "docs/a.txt"), cache().tree.keys)
    }

    @Test
    fun 完整空盘是可信checkpoint() {
        assertTrue(CloudTreeCache.trusted(emptyMap(), emptyMap(), null, "cursor").isTrusted())
    }

    private fun cache(): CloudTreeCache {
        val folder = DriveFile(id = "folder", name = "docs", parent = "root", mimeType = "application/vnd.huawei-apps.folder")
        val child = DriveFile(id = "child", name = "a.txt", parent = "folder", mimeType = "text/plain")
        return CloudTreeCache.trusted(
            tree = mapOf("docs" to folder, "docs/a.txt" to child),
            pathToId = mapOf("docs" to "folder", "docs/a.txt" to "child"),
            rootFolderId = "root",
            cursor = "c1",
        )
    }
}
