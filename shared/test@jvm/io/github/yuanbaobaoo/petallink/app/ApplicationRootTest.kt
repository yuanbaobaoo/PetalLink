package io.github.yuanbaobaoo.petallink.app

import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SetupPhase
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

    @Test
    fun 保存挂载配置后setupPhase从未配置进入待首同步() = runBlocking {
        val dataDir = Files.createTempDirectory("petallink-root-test-")
        val mountDir = Files.createTempDirectory("petallink-mount-test-")
        val root = ApplicationRoot(AppPaths(dataDir))
        try {
            withTimeout(5_000) {
                while (!root.viewModel.state.value.initialized) delay(10)
            }
            assertEquals(SetupPhase.NEEDS_SETUP, root.viewModel.state.value.setupPhase)

            val errors = root.viewModel.saveConfig(
                UserConfig(mountDir = mountDir.toString(), mountConfigured = true),
            )

            assertEquals(emptyList(), errors)
            assertTrue(root.viewModel.state.value.config.mountConfigured)
            assertEquals(SetupPhase.NEEDS_FIRST_SYNC, root.viewModel.state.value.setupPhase)
        } finally {
            root.close()
        }
    }
}
