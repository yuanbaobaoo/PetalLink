package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.sync.engine.UploadStability
import java.nio.file.Files
import java.nio.file.attribute.FileTime
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class JvmUploadStabilityProbeTest {
    @Test
    fun mtime_size_lsof都稳定才放行() = runBlocking {
        val file = Files.writeString(Files.createTempFile("petallink-stable-", ".txt"), "stable")
        Files.setLastModifiedTime(file, FileTime.fromMillis(1_000))
        val probe = JvmUploadStabilityProbe(
            busyChecker = LsofFileBusyChecker(LsofSampler { emptyList() }, pause = {}),
            nowMs = { 10_000 },
            pause = {},
        )
        assertEquals(UploadStability.STABLE, probe.check(file.toString()))
    }

    @Test
    fun 持续不稳定超过5分钟转Editing() = runBlocking {
        val file = Files.writeString(Files.createTempFile("petallink-editing-", ".txt"), "editing")
        var now = Files.getLastModifiedTime(file).toMillis()
        val probe = JvmUploadStabilityProbe(
            busyChecker = LsofFileBusyChecker(LsofSampler { listOf("Code") }, pause = {}),
            nowMs = { now },
            pause = {},
        )
        assertEquals(UploadStability.UNSTABLE, probe.check(file.toString()))
        now += 300_001
        assertEquals(UploadStability.EDITING, probe.check(file.toString()))
    }
}
