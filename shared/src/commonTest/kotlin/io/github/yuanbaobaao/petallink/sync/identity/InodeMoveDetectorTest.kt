package io.github.yuanbaobaao.petallink.sync.identity

import io.github.yuanbaobaao.petallink.mount.LocalFileEntry
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class InodeMoveDetectorTest {
    @Test
    fun 同inode新路径识别move并更新映射() = runBlocking {
        val store = MemoryStore(
            mutableMapOf(11UL to InodeRecord(11UL, "old/a.txt", "cloud-1", 1)),
        )
        val moves = InodeMoveDetector.detectMoves(
            listOf(entry(11UL, "new/a.txt")),
            store,
        )
        assertEquals(
            listOf(DetectedMove(11UL, "cloud-1", "old/a.txt", "new/a.txt")),
            moves,
        )
        assertEquals("new/a.txt", store.lookup(11UL)?.relativePath)
    }

    @Test
    fun copy的新inode不冒充move() = runBlocking {
        val store = MemoryStore(
            mutableMapOf(11UL to InodeRecord(11UL, "source.txt", "cloud-1", 1)),
        )
        val moves = InodeMoveDetector.detectMoves(
            listOf(entry(11UL, "source.txt"), entry(22UL, "copy.txt")),
            store,
        )
        assertEquals(emptyList(), moves)
    }

    private fun entry(inode: ULong, path: String) = LocalFileEntry(
        absolutePath = "/mount/$path",
        relativePath = path,
        inode = inode,
        size = 1,
        mtime = 1,
        isDirectory = false,
        isPlaceholder = false,
    )

    private class MemoryStore(
        private val records: MutableMap<ULong, InodeRecord>,
    ) : InodeIdentityStore {
        override suspend fun lookup(inode: ULong): InodeRecord? = records[inode]
        override suspend fun upsert(inode: ULong, relativePath: String, fileId: String) {
            records[inode] = InodeRecord(inode, relativePath, fileId, 2)
        }
        override suspend fun purgeMissing(seenInodes: Set<ULong>) {
            records.keys.retainAll(seenInodes)
        }
    }
}
