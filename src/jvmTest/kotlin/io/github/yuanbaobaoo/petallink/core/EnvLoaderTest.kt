package io.github.yuanbaobaoo.petallink.core

import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * OAuth 环境变量文件加载测试。
 */
class EnvLoaderTest {
    @Test
    fun loadEnvFile_读取当前工作目录的Env文件() {
        val directory = Files.createTempDirectory("petallink-env-loader-test")
        val envFile = directory.resolve(".env")
        try {
            Files.writeString(
                envFile,
                "HWCLOUD_CLIENT_ID=test-client-id\nHWCLOUD_CLIENT_SECRET=test-client-secret\n",
            )
            EnvLoader.buildClientId = ""
            EnvLoader.buildSecret = ""

            EnvLoader.loadEnvFile(directory)

            assertEquals("test-client-id", EnvLoader.resolvedClientId())
            assertEquals("test-client-secret", EnvLoader.resolvedClientSecret())
        } finally {
            Files.deleteIfExists(envFile)
            Files.deleteIfExists(directory)
        }
    }
}
