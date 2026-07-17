package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull

class JvmCloudTreeCheckpointStoreTest {
    @Test
    fun 单文件checkpoint可完整往返() = runBlocking {
        val file = Files.createTempDirectory("petallink-cloud-store-").resolve("cloudtree.json")
        val store = JvmCloudTreeCheckpointStore(file)
        val checkpoint = checkpoint("c1", "a")
        store.persist(checkpoint)
        val loaded = store.loadTrusted()
        assertEquals(checkpoint.cursor, loaded?.cursor)
        assertEquals(checkpoint.pathToId, loaded?.pathToId)
        assertEquals(checkpoint.tree["a"]?.parentFolder, loaded?.tree?.get("a")?.parentFolder)
        assertEquals(true, loaded?.isTrusted())
    }

    @Test
    fun rename后失败必须恢复旧checkpoint() = runBlocking {
        val file = Files.createTempDirectory("petallink-cloud-rollback-").resolve("cloudtree.json")
        JvmCloudTreeCheckpointStore(file).persist(checkpoint("old", "old.txt"))
        val failing = JvmCloudTreeCheckpointStore(file) { stage ->
            if (stage == CheckpointCommitStage.REPLACED) error("注入 parent fsync 前失败")
        }
        assertFailsWith<AppError.LocalIo> { failing.persist(checkpoint("new", "new.txt")) }
        assertEquals("old", JvmCloudTreeCheckpointStore(file).loadTrusted()?.cursor)
        assertEquals(setOf("old.txt"), JvmCloudTreeCheckpointStore(file).loadTrusted()?.tree?.keys)
    }

    @Test
    fun 损坏或不可信文件不得装载() = runBlocking {
        val file = Files.createTempDirectory("petallink-cloud-invalid-").resolve("cloudtree.json")
        Files.writeString(file, "{broken")
        assertNull(JvmCloudTreeCheckpointStore(file).loadTrusted())
    }

    private fun checkpoint(cursor: String, name: String): CloudTreeCache {
        val file = DriveFile(id = "id-$name", name = name, parent = "root", mimeType = "text/plain")
        return CloudTreeCache.trusted(mapOf(name to file), mapOf(name to "id-$name"), "root", cursor)
    }
}
