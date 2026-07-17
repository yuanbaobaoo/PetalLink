package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import kotlinx.serialization.json.Json
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class DriveParsersTest {
    @Test
    fun 兼容fileName和name及数字浮点字符串size() {
        val numeric = parse("""{"id":"1","fileName":"a","mimeType":"text/plain","size":12.9}""")
        val string = parse("""{"id":"2","name":"b","mimeType":"text/plain","size":"13"}""")
        assertEquals("a", numeric.name)
        assertEquals(12L, numeric.sizeBytes)
        assertEquals("b", string.name)
        assertEquals(13L, string.sizeBytes)
    }

    @Test
    fun 内容hash别名和父目录被规范化() {
        val file = parse(
            """{"category":"drive#file","id":"1","fileName":"a","mimeType":"text/plain","fileSha256":"abc","parentFolder":["root"]}""",
        )
        assertEquals("abc", file.contentHash)
        assertEquals("root", DriveParsers.singleParent(file))
    }

    @Test
    fun 四种完整文件夹MIME均可识别() {
        listOf(
            "application/vnd.huawei-apps.folder",
            "application/vnd.huawei-app.folder",
            "application/vnd.google-apps.folder",
            "application/x-folder",
        ).forEach { assertTrue(DriveParsers.isFolderMime(it)) }
    }

    @Test
    fun 严格列表拒绝缺失files和不完整条目() {
        assertFailsWith<AppError.Remote> {
            DriveParsers.parseFileListPage(Json.parseToJsonElement("{}"))
        }
        assertFailsWith<AppError.Remote> {
            DriveParsers.parseFileListPage(
                Json.parseToJsonElement("""{"files":[{"id":"1","fileName":"a"}]}"""),
            )
        }
    }

    @Test
    fun 单父目录拒绝零个和多个父目录() {
        assertFailsWith<AppError.Remote> {
            DriveParsers.singleParent(parse("""{"id":"1","fileName":"a","mimeType":"text/plain"}"""))
        }
        assertFailsWith<AppError.Remote> {
            DriveParsers.singleParent(
                parse("""{"id":"1","fileName":"a","mimeType":"text/plain","parentFolder":["a","b"]}"""),
            )
        }
    }

    private fun parse(raw: String): DriveFile =
        DriveParsers.parseDriveFileStrict(Json.parseToJsonElement(raw))
}
