package io.github.yuanbaobaao.petallink.compat

import io.github.yuanbaobaao.petallink.auth.TokenSerializer
import io.github.yuanbaobaao.petallink.config.JsonConfigStore
import io.github.yuanbaobaao.petallink.config.SortField
import io.github.yuanbaobaao.petallink.config.SortOrder
import io.github.yuanbaobaao.petallink.core.AppPaths
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class LegacyTauriCompatibilityTest {
    @Test
    fun 生产数据目录沿用原TauriBundleId() {
        val path = AppPaths.production().dataDir.toString()
        assertTrue(path.endsWith("Library/Application Support/io.github.yuanbaobaoo.PetalLink"))
    }

    @Test
    fun 原Tauri配置JSON可直接读取() {
        val dir = Files.createTempDirectory("petallink-legacy-config-")
        val file = dir.resolve("config.json")
        Files.writeString(file, """{
          "oauthRedirectUri":"http://127.0.0.1:9999/oauth/callback",
          "oauthCallbackPort":9999,
          "mountDir":"/tmp/PetalLinkLegacy",
          "mountConfigured":false,
          "concurrency":8,
          "pollIntervalSec":900,
          "debounceSec":3,
          "skipPatterns":[".DS_Store","*.part"],
          "sortField":"modifiedTime",
          "sortOrder":"descending"
        }""")
        val config = JsonConfigStore(file).load()!!
        assertEquals(8, config.concurrency)
        assertEquals(SortField.ModifiedTime, config.sortField)
        assertEquals(SortOrder.Descending, config.sortOrder)
    }

    @Test
    fun 原Rust明文Token布局可直接解析() {
        val access = "legacy-access".encodeToByteArray()
        val refresh = "legacy-refresh".encodeToByteArray()
        val type = "Bearer".encodeToByteArray()
        val scope = "openid".encodeToByteArray()
        val buffer = ByteBuffer.allocate(8 + access.size + 8 + refresh.size + 8 + 4 + type.size + 1 + 8 + scope.size)
            .order(ByteOrder.LITTLE_ENDIAN)
        buffer.putLong(access.size.toLong()).put(access)
        buffer.putLong(refresh.size.toLong()).put(refresh)
        buffer.putLong(1_900_000_000_000L)
        buffer.putInt(type.size).put(type)
        buffer.put(1.toByte()).putLong(scope.size.toLong()).put(scope)
        val token = TokenSerializer.deserialize(buffer.array())
        assertEquals("legacy-access", token.accessToken)
        assertEquals("legacy-refresh", token.refreshToken)
        assertEquals("openid", token.scope)
    }
}
