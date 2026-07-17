package io.github.yuanbaobaao.petallink.commands

import io.github.yuanbaobaao.petallink.config.AppConfig
import io.github.yuanbaobaao.petallink.config.ConfigStore
import io.github.yuanbaobaao.petallink.config.UserConfig
import io.github.yuanbaobaao.petallink.data.PetalLinkDb
import io.github.yuanbaobaao.petallink.data.SyncItem
import io.github.yuanbaobaao.petallink.mount.XattrAccess
import io.github.yuanbaobaao.petallink.sync.SyncStatus
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class JvmSyncStatusResolverTest {
    @Test
    fun 单项与批量状态严格返回原命令字符串且零字节用户文件不是占位符() = runBlocking {
        val root = Files.createTempDirectory("petallink-status-")
        val db = PetalLinkDb(root.resolve("state.db").toString())
        try {
            val real = Files.writeString(root.resolve("real.txt"), "content")
            val zero = Files.createFile(root.resolve("zero.txt"))
            val placeholder = Files.writeString(root.resolve("cloud.txt"), "edited placeholder")
            Files.createDirectory(root.resolve("folder"))
            val attrs = MemoryXattrs().apply {
                set(placeholder.toRealPath().toString(), AppConfig.XATTR_STATE, "placeholder".encodeToByteArray())
            }
            listOf(
                item("real", "real.txt"), item("zero", "zero.txt"), item("cloud", "cloud.txt"),
                item("folder", "folder", folder = true), item("missing", "missing.txt"),
            ).forEach { db.syncItems.upsert(it) }
            val resolver = JvmSyncStatusResolver(
                Store(UserConfig(mountDir = root.toString(), mountConfigured = true)), db.syncItems, attrs,
            )

            assertEquals("synced", resolver.resolveOne("real"))
            assertEquals("synced", resolver.resolveOne("zero"))
            assertEquals("placeholder", resolver.resolveOne("cloud"))
            assertEquals("folder", resolver.resolveOne("folder"))
            assertEquals("not_synced", resolver.resolveOne("missing"))
            assertEquals("not_synced", resolver.resolveOne("unknown"))
            assertEquals(
                mapOf("real" to "synced", "cloud" to "placeholder", "folder" to "folder", "unknown" to "not_synced"),
                resolver.resolveBatch(listOf("real", "cloud", "folder", "unknown")),
            )
        } finally {
            db.close()
        }
    }

    @Test
    fun 未配置目录时单项返回notSynced而批量按DB已同步状态降级() = runBlocking {
        val dir = Files.createTempDirectory("petallink-status-unconfigured-")
        val db = PetalLinkDb(dir.resolve("state.db").toString())
        try {
            db.syncItems.upsert(item("synced", "a.txt", status = SyncStatus.SYNCED))
            db.syncItems.upsert(item("failed", "b.txt", status = SyncStatus.FAILED))
            val resolver = JvmSyncStatusResolver(Store(UserConfig()), db.syncItems, MemoryXattrs())
            assertEquals("not_synced", resolver.resolveOne("synced"))
            assertEquals(
                mapOf("synced" to "synced", "failed" to "not_synced"),
                resolver.resolveBatch(listOf("synced", "failed")),
            )
        } finally {
            db.close()
        }
    }

    private fun item(id: String, path: String, folder: Boolean = false, status: Int = SyncStatus.SYNCED) = SyncItem(
        id, path, "root", path.substringAfterLast('/'), folder, 0, 0, null, 1, 2, 3, status, null,
    )

    private class Store(private var value: UserConfig) : ConfigStore {
        override fun load() = value
        override fun save(config: UserConfig) { value = config }
    }

    private class MemoryXattrs : XattrAccess {
        private val values = mutableMapOf<Pair<String, String>, ByteArray>()
        override fun get(path: String, name: String) = values[path to name]?.copyOf()
        override fun set(path: String, name: String, value: ByteArray) { values[path to name] = value.copyOf() }
        override fun remove(path: String, name: String) { values.remove(path to name) }
    }
}
