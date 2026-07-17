package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.auth.TokenPair
import io.github.yuanbaobaao.petallink.drive.ChangesApi
import io.github.yuanbaobaao.petallink.drive.DriveClient
import io.github.yuanbaobaao.petallink.drive.FilesApi
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFails
import kotlin.test.assertTrue

class BfsCloudTreeRefresherTest {
    @Test
    fun 全量严格按startCursor_BFS_replay后才提交() = runBlocking {
        val calls = mutableListOf<String>()
        val engine = MockEngine { request ->
            val url = request.url.toString()
            calls += url
            val body = when {
                url.contains("getStartCursor") -> """{"category":"drive#startCursor","startCursor":"s0"}"""
                url.contains("/changes?") -> """{"category":"drive#changeList","changes":[],"newStartCursor":"c1"}"""
                url.contains("folder%27") -> fileList("""{"category":"drive#file","id":"child","fileName":"a.txt","mimeType":"text/plain","parentFolder":["folder"]}""")
                else -> fileList("""{"category":"drive#file","id":"folder","fileName":"docs","mimeType":"application/vnd.huawei-apps.folder","parentFolder":["root"]}""")
            }
            respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))
        }
        val store = MemoryStore()
        val refresher = refresher(engine, store)

        val cache = refresher.refreshFull()

        assertEquals(listOf("docs", "docs/a.txt"), cache.tree.keys.sorted())
        assertEquals("c1", cache.cursor)
        assertTrue(cache.isTrusted())
        assertEquals(cache, store.value)
        assertTrue(calls.first().contains("getStartCursor"))
        assertTrue(calls.last().contains("/changes?"))
    }

    @Test
    fun 增量在clone上改名子树并原子提交() = runBlocking {
        val old = baseCache()
        val engine = MockEngine {
            respond(
                """{"category":"drive#changeList","changes":[{"category":"drive#change","type":"File","fileId":"folder","deleted":false,"file":{"category":"drive#file","id":"folder","fileName":"renamed","mimeType":"application/vnd.huawei-apps.folder","parentFolder":["root"]}}],"newStartCursor":"c2"}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }
        val store = MemoryStore(old)

        val result = refresher(engine, store).refreshIncremental("c1")

        assertEquals("c2", result.cursor)
        assertEquals("child", result.pathToId["renamed/a.txt"])
        assertTrue("docs" in old.tree, "原 cache 不得被就地修改")
    }

    @Test
    fun 失效cursor保留旧盘并回退可信全量() = runBlocking {
        var changesCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            when {
                url.contains("/changes?") && changesCalls++ == 0 -> respond("gone", HttpStatusCode.Gone)
                url.contains("getStartCursor") -> respondJson("""{"category":"drive#startCursor","startCursor":"s-new"}""")
                url.contains("/files?") -> respondJson(fileList())
                else -> respondJson("""{"category":"drive#changeList","changes":[],"newStartCursor":"c-new"}""")
            }
        }
        val store = MemoryStore(baseCache())

        val result = refresher(engine, store).refreshIncremental("c1")

        assertEquals("c-new", result.cursor)
        assertTrue(result.tree.isEmpty())
        assertEquals(result, store.value)
    }

    @Test
    fun 根目录parent最高频平局必须failClosed() = runBlocking {
        val engine = MockEngine { request ->
            val url = request.url.toString()
            val body = when {
                url.contains("getStartCursor") -> """{"category":"drive#startCursor","startCursor":"s0"}"""
                else -> fileList(
                    """{"category":"drive#file","id":"a","fileName":"a","mimeType":"text/plain","parentFolder":["root-a"]}""",
                    """{"category":"drive#file","id":"b","fileName":"b","mimeType":"text/plain","parentFolder":["root-b"]}""",
                )
            }
            respondJson(body)
        }
        assertFails { refresher(engine, MemoryStore()).refreshFull() }
        Unit
    }

    @Test
    fun Changes中途失败不提交部分BFS或推进cursor() = runBlocking {
        val old = baseCache()
        val store = MemoryStore(old)
        val engine = MockEngine { request ->
            val url = request.url.toString()
            when {
                url.contains("getStartCursor") -> respondJson("""{"category":"drive#startCursor","startCursor":"s0"}""")
                url.contains("/files?") -> respondJson(fileList())
                else -> respond("failed", HttpStatusCode.InternalServerError)
            }
        }
        assertFails { refresher(engine, store).refreshFull() }
        assertEquals(old, store.value)
        assertEquals(0, store.persistCount)
    }

    @Test
    fun 连续增量达阈值后强制全量() = runBlocking {
        var getStartCalls = 0
        var changesCalls = 0
        val engine = MockEngine { request ->
            val url = request.url.toString()
            when {
                url.contains("getStartCursor") -> {
                    getStartCalls++
                    respondJson("""{"category":"drive#startCursor","startCursor":"s-full"}""")
                }
                url.contains("/files?") -> respondJson(fileList())
                else -> {
                    changesCalls++
                    val cursor = if (changesCalls == 1) "c2" else "c3"
                    respondJson("""{"category":"drive#changeList","changes":[],"newStartCursor":"$cursor"}""")
                }
            }
        }
        val store = MemoryStore(baseCache())
        val client = DriveClient(HttpClient(engine), { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        val refresher = BfsCloudTreeRefresher(
            FilesApi(client, "https://example.test/drive/v1"),
            ChangesApi(client, "https://example.test/drive/v1"),
            store,
            forcedFullThreshold = 1,
        )
        assertEquals("c2", refresher.refreshIncremental("c1").cursor)
        assertEquals("c3", refresher.refreshIncremental("c2").cursor)
        assertEquals(1, getStartCalls)
    }

    private fun refresher(engine: MockEngine, store: CloudTreeCheckpointStore): BfsCloudTreeRefresher {
        val client = DriveClient(HttpClient(engine), { "token" }, { TokenPair("new", "refresh", Long.MAX_VALUE) })
        return BfsCloudTreeRefresher(
            FilesApi(client, "https://example.test/drive/v1"),
            ChangesApi(client, "https://example.test/drive/v1"),
            store,
        )
    }

    private fun fileList(vararg files: String): String =
        """{"category":"drive#fileList","files":[${files.joinToString(",")}]}"""

    private fun io.ktor.client.engine.mock.MockRequestHandleScope.respondJson(body: String) =
        respond(body, HttpStatusCode.OK, headersOf(HttpHeaders.ContentType, "application/json"))

    private fun baseCache(): CloudTreeCache = CloudTreeCache.trusted(
        mapOf(
            "docs" to io.github.yuanbaobaao.petallink.drive.DriveFile(id = "folder", name = "docs", parent = "root", mimeType = "application/vnd.huawei-apps.folder"),
            "docs/a.txt" to io.github.yuanbaobaao.petallink.drive.DriveFile(id = "child", name = "a.txt", parent = "folder", mimeType = "text/plain"),
        ),
        mapOf("docs" to "folder", "docs/a.txt" to "child"),
        "root",
        "c1",
    )

    private class MemoryStore(initial: CloudTreeCache? = null) : CloudTreeCheckpointStore {
        var value = initial
        var persistCount = 0
        override suspend fun loadTrusted(): CloudTreeCache? = value
        override suspend fun persist(checkpoint: CloudTreeCache) {
            checkpoint.validateTrusted()
            value = checkpoint
            persistCount++
        }
        override suspend fun discardUncommitted() = Unit
    }
}
