package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.sync.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

/** SyncEngine + BfsCloudTreeRefresher 单测 */
class SyncEngineTest {

    @Test
    fun CycleCoordinator八步触发位集正确() {
        val startup = CycleTrigger.requestFor(CycleTrigger.STARTUP_RESUME)
        assertTrue(startup.contains(CycleRequest.STARTUP))
        assertTrue(startup.contains(CycleRequest.LOCAL_RESCAN))
        assertTrue(startup.contains(CycleRequest.ONLINE_RECOVERY))
    }

    @Test
    fun CloudTreeCache空树可信() {
        val cache = CloudTreeCache(
            tree = emptyMap(), pathToId = emptyMap(),
            rootFolderId = null, cursor = "c1", complete = true,
        )
        assertTrue(cache.isTrusted())
    }

    @Test
    fun CloudTreeRefresh_detectRootFolderId最高频返回() {
        val files = listOf(
            DriveFile(id = "1", parent = "rootId"),
            DriveFile(id = "2", parent = "rootId"),
            DriveFile(id = "3", parent = "other"),
        )
        assertEquals("rootId", CloudTreeRefresh.detectRootFolderId(files))
    }

    @Test
    fun CloudTreeRefresh平局返回null() {
        val files = listOf(
            DriveFile(id = "1", parent = "a"),
            DriveFile(id = "2", parent = "b"),
        )
        assertEquals(null, CloudTreeRefresh.detectRootFolderId(files))
    }
}