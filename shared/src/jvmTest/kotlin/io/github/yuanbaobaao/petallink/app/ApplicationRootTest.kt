package io.github.yuanbaobaao.petallink.app

import io.github.yuanbaobaao.petallink.core.AppPaths
import java.nio.file.Files
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class ApplicationRootTest {
    @Test
    fun 所有运行文件只写入注入目录且关闭幂等() = runBlocking {
        val dataDir = Files.createTempDirectory("petallink-root-test-")
        val root = ApplicationRoot(AppPaths(dataDir))
        try {
            withTimeout(5_000) {
                while (!root.viewModel.state.value.initialized) delay(10)
            }
            assertEquals(dataDir, root.paths.dataDir)
            assertTrue(Files.exists(root.paths.databaseFile))
            assertTrue(Files.exists(root.paths.logsDir))
        } finally {
            root.close()
            root.close()
        }
    }
}
