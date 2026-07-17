package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.config.AppConfig
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.StandardOpenOption
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class LsofFileBusyCheckerTest {
    @Test
    fun parser只提取合法pid下的command并去重() {
        val output = """
            cignored
            p123
            cjava
            f10u
            p456
            cmdworker_shared
            p789
            cjava
        """.trimIndent()
        assertEquals(listOf("java", "mdworker_shared"), LsofParser.commands(output))
    }

    @Test
    fun 全部白名单进程不判busy() = runBlocking {
        val checker = LsofFileBusyChecker(
            sampler = LsofSampler { AppConfig.STABILITY_LSOF_WHITELIST },
            pause = {},
        )
        assertFalse(checker.check(Files.createTempFile("petallink-lsof-white-", ".tmp")).busy)
    }

    @Test
    fun 短暂占用在二次采样消失则放行() = runBlocking {
        var samples = 0
        val checker = LsofFileBusyChecker(
            sampler = LsofSampler { if (samples++ == 0) listOf("Editor") else emptyList() },
            pause = {},
        )
        val result = checker.check(Files.createTempFile("petallink-lsof-transient-", ".tmp"))
        assertFalse(result.busy)
        assertEquals(2, samples)
    }

    @Test
    fun 持续非白名单占用在二次采样后仍busy() = runBlocking {
        var samples = 0
        val checker = LsofFileBusyChecker(
            sampler = LsofSampler { samples++; listOf("Code") },
            pause = {},
        )
        val result = checker.check(Files.createTempFile("petallink-lsof-busy-", ".tmp"))
        assertTrue(result.busy)
        assertEquals(listOf("Code"), result.processes)
        assertEquals(2, samples)
    }

    @Test
    fun 真实lsof可看到当前JVM打开的文件() {
        if (!System.getProperty("os.name").contains("Mac", ignoreCase = true)) return
        val file = Files.createTempFile("petallink-lsof-native-", ".tmp")
        FileChannel.open(file, StandardOpenOption.WRITE).use {
            val commands = LsofFileBusyChecker.sampleWithLsof(file)
            assertTrue(commands.isNotEmpty(), "lsof 未返回当前 JVM 进程")
        }
    }
}
