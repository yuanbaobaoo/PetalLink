package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.auth.TokenPair
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.engine.mock.toByteArray
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.HttpMethod
import io.ktor.http.headersOf
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class FilesApiHttpTest {
    @Test
    fun list使用真实root和parent查询且cursor循环失败() { runBlocking {
        val requests = mutableListOf<String>()
        val engine = MockEngine { request ->
            requests += request.url.toString()
            respond(
                """{"category":"drive#fileList","files":[],"nextCursor":"same"}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }
        val api = api(engine)
        val first = api.listFiles(null)
        assertEquals("same", first.second)
        assertTrue(requests.first().contains("queryParam=%27root%27"))
        assertFalse(requests.first().contains("%27id%27"))
        assertFailsWith<io.github.yuanbaobaoo.petallink.AppError.Remote> {
            api.listAllFiles("folder")
        }
    } }

    @Test
    fun 中文和emoji创建请求使用ASCII转义且响应必须匹配名称父目录和类型() { runBlocking {
        var requestBody = ByteArray(0)
        val engine = MockEngine { request ->
            if (request.method == HttpMethod.Get) {
                respond(
                    """{"category":"drive#fileList","files":[]}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            } else {
                requestBody = request.body.toByteArray()
                respond(
                    """{"category":"drive#file","id":"f1","fileName":"花瓣🌸","mimeType":"application/vnd.huawei-apps.folder","parentFolder":["root"],"size":0}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            }
        }
        val file = api(engine).createFile("花瓣🌸", null, true)
        assertEquals("f1", file.id)
        val body = requestBody.decodeToString()
        assertFalse(body.contains("花瓣🌸"))
        assertTrue(body.contains("\\u82b1\\u74e3\\ud83c\\udf38"))
    } }

    @Test
    fun delete必须核验响应id和recycled() { runBlocking {
        val engine = MockEngine {
            respond(
                """{"category":"drive#file","id":"wrong","fileName":"a","mimeType":"text/plain","recycled":false}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }
        assertFailsWith<IllegalArgumentException> { api(engine).deleteFile("expected") }
    } }

    @Test
    fun create响应丢失后通过父目录唯一结果收敛且不重复POST() { runBlocking {
        var gets = 0
        var posts = 0
        val engine = MockEngine { request ->
            when (request.method) {
                HttpMethod.Get -> {
                    gets++
                    val files = if (gets == 1) "[]" else
                        """[{"category":"drive#file","id":"created","fileName":"folder","mimeType":"application/vnd.huawei-apps.folder","parentFolder":["root"]}]"""
                    respond(
                        """{"category":"drive#fileList","files":$files}""",
                        HttpStatusCode.OK,
                        headersOf(HttpHeaders.ContentType, "application/json"),
                    )
                }
                else -> {
                    posts++
                    respond("gateway lost", HttpStatusCode.BadGateway)
                }
            }
        }
        val result = api(engine).createFile("folder", null, true)
        assertEquals("created", result.id)
        assertEquals(1, posts)
        assertEquals(2, gets)
    } }

    @Test
    fun rename响应丢失后GET确认新名称() { runBlocking {
        var patches = 0
        val engine = MockEngine { request ->
            if (request.method == HttpMethod.Patch) {
                patches++
                respond("lost", HttpStatusCode.BadGateway)
            } else respond(
                """{"category":"drive#file","id":"f1","fileName":"new","mimeType":"text/plain","parentFolder":["root"]}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }
        assertEquals("new", api(engine).updateFile("f1", "new").name)
        assertEquals(1, patches)
    } }

    @Test
    fun move响应丢失后GET确认目标父目录() { runBlocking {
        var gets = 0
        var patches = 0
        val engine = MockEngine { request ->
            if (request.method == HttpMethod.Patch) {
                patches++
                respond("lost", HttpStatusCode.BadGateway)
            } else {
                gets++
                val parent = if (gets == 1) "old" else "new"
                respond(
                    """{"category":"drive#file","id":"f1","fileName":"a","mimeType":"text/plain","parentFolder":["$parent"]}""",
                    HttpStatusCode.OK,
                    headersOf(HttpHeaders.ContentType, "application/json"),
                )
            }
        }
        assertEquals("new", DriveParsers.singleParent(api(engine).moveFile("f1", "old", "new")))
        assertEquals(1, patches)
    } }

    @Test
    fun delete响应丢失后GET确认recycled() { runBlocking {
        var patches = 0
        val engine = MockEngine { request ->
            if (request.method == HttpMethod.Patch) {
                patches++
                respond("lost", HttpStatusCode.BadGateway)
            } else respond(
                """{"category":"drive#file","id":"f1","fileName":"a","mimeType":"text/plain","parentFolder":["root"],"recycled":true}""",
                HttpStatusCode.OK,
                headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }
        api(engine).deleteFile("f1")
        assertEquals(1, patches)
    } }

    private fun api(engine: MockEngine): FilesApi {
        val client = DriveClient(
            HttpClient(engine),
            tokenProvider = { "token" },
            tokenRefresher = { TokenPair("new", "refresh", 3600) },
        )
        return FilesApi(client, "https://example.test/drive/v1")
    }
}
