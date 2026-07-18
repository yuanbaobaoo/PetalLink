package io.github.yuanbaobaoo.petallink.sync

import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * ConflictResolver 单测（对标 src/sync/conflict.rs 副本命名）。
 */
class ConflictResolverTest {

    @Test
    fun 副本命名_带扩展名() {
        val name = ConflictResolver.copyName("doc.txt", ConflictResolver.ConflictSide.LOCAL, "2026-07-16 10-30-00")
        assertEquals("doc (本地副本 2026-07-16 10-30-00).txt", name)
    }

    @Test
    fun 副本命名_无扩展名() {
        val name = ConflictResolver.copyName("README", ConflictResolver.ConflictSide.CLOUD, "2026-07-16 10-30-00")
        assertEquals("README (云端副本 2026-07-16 10-30-00)", name)
    }

    @Test
    fun 副本命名_带序号() {
        val name = ConflictResolver.copyName("doc.txt", ConflictResolver.ConflictSide.LOCAL, "2026-07-16 10-30-00", sequence = 1)
        assertEquals("doc (本地副本 2026-07-16 10-30-00) (1).txt", name)
    }

    @Test
    fun 副本命名_多点扩展名取最后() {
        val name = ConflictResolver.copyName("archive.tar.gz", ConflictResolver.ConflictSide.LOCAL, "2026-07-16 10-30-00")
        assertEquals("archive.tar (本地副本 2026-07-16 10-30-00).gz", name)
    }

    @Test
    fun 本地领先云端超过阈值时本地胜出() {
        val resolution = ConflictResolver.resolve(161_001L, 100_000L)
        assertEquals(ConflictResolver.ConflictSide.LOCAL, resolution.winner)
        assertEquals(ConflictResolver.ConflictSide.CLOUD, resolution.loser)
        assertEquals(100_000L, resolution.loserTimestampMs)
    }

    @Test
    fun 阈值边界和时钟接近时云端胜出() {
        val resolution = ConflictResolver.resolve(160_000L, 100_000L)
        assertEquals(ConflictResolver.ConflictSide.CLOUD, resolution.winner)
        assertEquals(ConflictResolver.ConflictSide.LOCAL, resolution.loser)
        assertEquals(160_000L, resolution.loserTimestampMs)
    }
}
