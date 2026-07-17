package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.auth.TokenPair
import io.github.yuanbaobaoo.petallink.config.ConfigStore
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.drive.ChangesApi
import io.github.yuanbaobaoo.petallink.drive.DriveClient
import io.github.yuanbaobaoo.petallink.drive.FilesApi
import io.github.yuanbaobaoo.petallink.drive.UploadApi
import io.github.yuanbaobaoo.petallink.mount.JvmPlaceholderManager
import io.github.yuanbaobaoo.petallink.mount.MacXattrAccess
import io.github.yuanbaobaoo.petallink.sync.engine.JvmCloudTreeCheckpointStore
import io.github.yuanbaobaoo.petallink.sync.engine.StatusAggregator
import io.github.yuanbaobaoo.petallink.sync.engine.UploadStability
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpMethod
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.delay
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertTrue
import kotlin.test.assertFalse

class JvmSyncRuntimeIntegrationTest {
    @Test
    fun 临时目录与MockDrive完成一次双向同步周期() = runBlocking {
        val workspace = Files.createTempDirectory("petallink-cycle-")
        val mount = Files.createDirectory(workspace.resolve("mount"))
        val data = Files.createDirectory(workspace.resolve("data"))
        Files.writeString(mount.resolve("local.txt"), "local")
        var uploadCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Post && url.contains("uploadType=multipart") -> {
                    uploadCalls++
                    """{"category":"drive#file","id":"uploaded","fileName":"local.txt","mimeType":"text/plain","parentFolder":["root"],"size":"5"}"""
                }
                url.contains("/files?") ->
                    """{"category":"drive#fileList","files":[{"category":"drive#file","id":"remote","fileName":"remote.txt","mimeType":"text/plain","parentFolder":["root"],"size":"6"}]}"""
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = object : ConfigStore {
            private var value = UserConfig(mountDir = mount.toString(), mountConfigured = true)
            override fun load(): UserConfig = value
            override fun save(config: UserConfig) { value = config }
        }
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        val runtime = JvmSyncRuntime(
            paths, config, db,
            FilesApi(client, "https://example.test/drive/v1"),
            ChangesApi(client, "https://example.test/drive/v1"),
            UploadApi(client, "https://example.test/upload/drive/v1"),
            io.github.yuanbaobaoo.petallink.drive.DownloadApi(client, "https://example.test/drive/v1"),
            StatusAggregator(),
            stabilityFactory = { { UploadStability.STABLE } },
        )
        try {
            val firstRefresh = runtime.manualRefresh()
            assertIs<AppResult.Ok<Unit>>(firstRefresh, firstRefresh.toString())
            assertEquals(1, uploadCalls)
            val placeholder = mount.resolve("remote.txt")
            assertTrue(Files.exists(placeholder))
            assertTrue(JvmPlaceholderManager(mount, MacXattrAccess).isPlaceholder(placeholder.toString()))
            assertEquals("local.txt", db.syncItems.findByFileId("uploaded")?.localPath)
            assertEquals("remote.txt", db.syncItems.findByFileId("remote")?.localPath)
            val checkpoint = assertNotNull(
                JvmCloudTreeCheckpointStore(paths.cloudTreeCheckpoint(mount)).loadTrusted(),
            )
            assertEquals(setOf("local.txt", "remote.txt"), checkpoint.tree.keys)
            assertEquals("c1", checkpoint.cursor)
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    @Test
    fun 切换到空目录不会用旧基线删除云端文件() = runBlocking {
        val workspace = Files.createTempDirectory("petallink-switch-mount-")
        val mountA = Files.createDirectory(workspace.resolve("mount-a"))
        val mountB = Files.createDirectory(workspace.resolve("mount-b"))
        val data = Files.createDirectory(workspace.resolve("data"))
        Files.writeString(mountA.resolve("local.txt"), "local")
        var listCalls = 0
        var deleteCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                request.method == HttpMethod.Post && url.contains("uploadType=multipart") ->
                    """{"category":"drive#file","id":"uploaded","fileName":"local.txt","mimeType":"text/plain","parentFolder":["root"],"size":"5"}"""
                request.method == HttpMethod.Delete -> {
                    deleteCalls++
                    "{}"
                }
                url.contains("/files?") -> {
                    listCalls++
                    val uploaded = if (listCalls > 1) {
                        """,{"category":"drive#file","id":"uploaded","fileName":"local.txt","mimeType":"text/plain","parentFolder":["root"],"size":"5"}"""
                    } else ""
                    """{"category":"drive#fileList","files":[{"category":"drive#file","id":"remote","fileName":"remote.txt","mimeType":"text/plain","parentFolder":["root"],"size":"6"}$uploaded]}"""
                }
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val configStore = object : ConfigStore {
            var value = UserConfig(mountDir = mountA.toString(), mountConfigured = true)
            override fun load(): UserConfig = value
            override fun save(config: UserConfig) { value = config }
        }
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        val runtime = JvmSyncRuntime(
            paths, configStore, db,
            FilesApi(client, "https://example.test/drive/v1"),
            ChangesApi(client, "https://example.test/drive/v1"),
            UploadApi(client, "https://example.test/upload/drive/v1"),
            io.github.yuanbaobaoo.petallink.drive.DownloadApi(client, "https://example.test/drive/v1"),
            StatusAggregator(),
            stabilityFactory = { { UploadStability.STABLE } },
        )
        try {
            val firstRefresh = runtime.manualRefresh()
            assertIs<AppResult.Ok<Unit>>(firstRefresh, firstRefresh.toString())
            val oldCheckpoint = paths.cloudTreeCheckpoint(mountA)
            assertTrue(Files.exists(oldCheckpoint))

            val previous = configStore.value
            val current = previous.copy(mountDir = mountB.toString())
            runtime.prepareConfigurationChange()
            configStore.save(current)
            runtime.configurationChanged(previous, current)
            repeat(200) {
                if (!Files.exists(oldCheckpoint) && db.syncItems.selectAll().isEmpty()) return@repeat
                delay(10)
            }
            assertFalse(Files.exists(oldCheckpoint))
            assertTrue(db.syncItems.selectAll().isEmpty())

            assertIs<AppResult.Ok<Unit>>(runtime.manualRefresh())
            assertEquals(0, deleteCalls)
            assertTrue(JvmPlaceholderManager(mountB, MacXattrAccess).isPlaceholder(mountB.resolve("remote.txt").toString()))
            assertTrue(JvmPlaceholderManager(mountB, MacXattrAccess).isPlaceholder(mountB.resolve("local.txt").toString()))
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }

    @Test
    fun 目录递归同步立即返回并在后台把占位符下载为真实内容() = runBlocking {
        val workspace = Files.createTempDirectory("petallink-folder-sync-")
        val mount = Files.createDirectory(workspace.resolve("mount"))
        val data = Files.createDirectory(workspace.resolve("data"))
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                request.url.parameters["form"] == "content" -> "hello"
                url.contains("getStartCursor") ->
                    """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") ->
                    """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                url.contains("/files/remote?fields=*") ->
                    """{"category":"drive#file","id":"remote","fileName":"a.txt","mimeType":"text/plain","parentFolder":["folder"],"size":"5","editedTime":"2026-01-01T00:00:00Z"}"""
                url.contains("/files?") && request.url.parameters["queryParam"]?.contains("folder") == true ->
                    """{"category":"drive#fileList","files":[{"category":"drive#file","id":"remote","fileName":"a.txt","mimeType":"text/plain","parentFolder":["folder"],"size":"5","editedTime":"2026-01-01T00:00:00Z"}]}"""
                url.contains("/files?") ->
                    """{"category":"drive#fileList","files":[{"category":"drive#file","id":"folder","fileName":"docs","mimeType":"application/vnd.huawei-apps.folder","parentFolder":["root"],"size":"0"}]}"""
                else -> error("未预期请求: ${request.method.value} $url")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val httpClient = HttpClient(engine)
        val client = DriveClient(httpClient, { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val config = object : ConfigStore {
            override fun load() = UserConfig(mountDir = mount.toString(), mountConfigured = true)
            override fun save(config: UserConfig) = Unit
        }
        val paths = AppPaths(data)
        val db = PetalLinkDb(paths.databaseFile.toString())
        val runtime = JvmSyncRuntime(
            paths, config, db,
            FilesApi(client, "https://example.test/drive/v1"),
            ChangesApi(client, "https://example.test/drive/v1"),
            UploadApi(client, "https://example.test/upload/drive/v1"),
            io.github.yuanbaobaoo.petallink.drive.DownloadApi(client, "https://example.test/drive/v1"),
            StatusAggregator(),
            stabilityFactory = { { UploadStability.STABLE } },
        )
        try {
            runtime.start()
            var accepted = false
            repeat(300) {
                if (!accepted) accepted = runtime.enqueueFolderSync("folder", "docs")
                if (!accepted) delay(10)
            }
            assertTrue(accepted)
            val downloaded = mount.resolve("docs/a.txt")
            repeat(500) {
                if (!Files.isRegularFile(downloaded) || Files.size(downloaded) != 5L) delay(10)
            }
            assertEquals("hello", Files.readString(downloaded))
            assertFalse(JvmPlaceholderManager(mount, MacXattrAccess).isPlaceholder(downloaded.toString()))
            // 文件落地与 folderSyncProgress 状态发布是两个异步步骤：文件就绪后仍需等待进度发布到 done。
            repeat(500) {
                if (runtime.folderSyncProgress().value != FolderSyncProgress(1, 1)) delay(10)
            }
            assertEquals(FolderSyncProgress(1, 1), runtime.folderSyncProgress().value)
        } finally {
            runtime.close()
            db.close()
            httpClient.close()
        }
    }
}
