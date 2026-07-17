package io.github.yuanbaobaao.petallink.config

import java.nio.file.Files
import kotlin.io.path.readText
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFails
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlinx.serialization.json.Json

class JsonConfigStoreTest {
    @Test
    fun 缺失配置返回空且不创建真实目录() {
        val dir = Files.createTempDirectory("petallink-config-test-")
        val file = dir.resolve("config.json")
        assertNull(JsonConfigStore(file).load())
        assertFalse(Files.exists(file))
    }

    @Test
    fun 配置可保存并完整恢复() {
        val dir = Files.createTempDirectory("petallink-config-test-")
        val file = dir.resolve("config.json")
        val expected = UserConfig(
            oauthRedirectUri = "http://127.0.0.1:9999/oauth/callback",
            concurrency = 9,
            pollIntervalSec = 120,
            skipPatterns = listOf(".DS_Store", "*.part"),
            sortField = SortField.ModifiedTime,
            sortOrder = SortOrder.Descending,
        )
        val store = JsonConfigStore(file)
        store.save(expected)
        assertEquals(expected, store.load())
    }

    @Test
    fun 损坏配置向调用方传播错误() {
        val dir = Files.createTempDirectory("petallink-config-test-")
        val file = dir.resolve("config.json")
        Files.writeString(file, "{not-json")
        assertFails { JsonConfigStore(file).load() }
    }

    @Test
    fun 配置路径父级不是目录时保存失败会传播() {
        val dir = Files.createTempDirectory("petallink-config-test-")
        val blocker = dir.resolve("not-a-directory")
        Files.writeString(blocker, "block")
        assertFails { JsonConfigStore(blocker.resolve("config.json")).save(UserConfig()) }
    }

    @Test
    fun 旧版危险轮询值会迁移并回写() {
        val dir = Files.createTempDirectory("petallink-config-test-")
        val file = dir.resolve("config.json")
        Files.writeString(file, """{"pollIntervalSec":10,"debounceSec":30}""")
        val loaded = JsonConfigStore(file).load()!!
        assertEquals(60L, loaded.pollIntervalSec)
        assertEquals(3L, loaded.debounceSec)
        val persisted = Json.decodeFromString(UserConfig.serializer(), file.readText())
        assertEquals(60L, persisted.pollIntervalSec)
    }

    @Test
    fun 主目录缩写按userHome展开且禁止主目录本身() {
        val originalHome = System.getProperty("user.home")
        val fakeHome = Files.createTempDirectory("petallink-fake-home-")
        val configFile = Files.createTempDirectory("petallink-config-test-").resolve("config.json")
        try {
            System.setProperty("user.home", fakeHome.toString())
            val store = JsonConfigStore(configFile)
            store.save(UserConfig(mountDir = "~/Drive", mountConfigured = true))
            assertTrue(Files.isDirectory(fakeHome.resolve("Drive")))
            assertFails { store.save(UserConfig(mountDir = "~", mountConfigured = true)) }
        } finally {
            System.setProperty("user.home", originalHome)
        }
    }

    @Test
    fun 导入解析与启动加载使用同一套迁移规则() {
        val file = Files.createTempDirectory("petallink-config-import-").resolve("config.json")
        val parsed = JsonConfigStore(file).parseImport("""{"pollIntervalSec":10,"debounceSec":30}""")
        assertEquals(60L, parsed.pollIntervalSec)
        assertEquals(3L, parsed.debounceSec)
        assertFalse(Files.exists(file))
    }
}
