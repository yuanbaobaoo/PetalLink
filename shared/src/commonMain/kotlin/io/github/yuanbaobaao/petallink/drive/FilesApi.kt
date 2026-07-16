package io.github.yuanbaobaao.petallink.drive

import io.github.yuanbaobaao.petallink.auth.Pkce
import io.ktor.client.request.header
import io.ktor.client.request.setBody
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

/**
 * 华为 Files API（对标 src/drive/files_api.rs）。
 *
 * 详见 docs/03 §Files API、docs/10 阶段 2 item 9。
 * 踩坑：parentFolder 用 queryParam 语法（'id' in parentFolder），非参数。
 */
class FilesApi(
    private val client: DriveClient,
    private val base: String = DriveConstants.DRIVE_API_BASE,
) {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    /** 列出文件（分页） */
    suspend fun listFiles(
        parentId: String?,
        pageSize: Int = 100,
        nextCursor: String? = null,
    ): Pair<List<DriveFile>, String?> {
        val params = mutableListOf("pageSize" to pageSize.toString())
        // 踩坑 5：List 用 queryParam='id' in parentFolder，非 parentFolder 参数
        if (parentId != null) {
            params += "queryParam" to "'id' in parentFolder"
        }
        nextCursor?.let { params += "cursor" to it }
        val qs = params.joinToString("&") { "${it.first}=${it.second}" }

        val resp = client.executeWithRetry(
            HttpMethod.Get, "$base/files?$qs", HttpSemantics.READ,
        )
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        val files = body["files"]?.jsonArray?.map { parseDriveFile(it.jsonObject) } ?: emptyList()
        val cursor = body["nextCursor"]?.jsonPrimitive?.contentOrNull
        return files to cursor
    }

    /** 获取单个文件（fields=* 全字段） */
    suspend fun getFile(fileId: String): DriveFile {
        val resp = client.executeWithRetry(
            HttpMethod.Get,
            "$base/files/${Pkce.enc(fileId)}?fields=*",
            HttpSemantics.READ,
        )
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        return parseDriveFile(body)
    }

    /** 创建文件/文件夹 */
    suspend fun createFile(name: String, parentFolder: String?, isFolder: Boolean): DriveFile {
        val meta = buildJsonObject {
            // 踩坑 4：中文文件名在 application/json 需 ascii 转义
            put("fileName", AsciiJson.escape(name))
            if (parentFolder != null) putJsonArray("parentFolder") { add(parentFolder) }
            if (isFolder) put("mimeType", "application/vnd.huawei-apps.folder")
        }
        val resp = client.executeWithRetry(
            HttpMethod.Post, "$base/files", HttpSemantics.WRITE,
        ) {
            contentType(ContentType.Application.Json)
            setBody(meta.toString())
        }
        // 踩坑 8：写操作必须 200（require_official_write_ok）
        require(resp.status.value == 200) {
            throw io.github.yuanbaobaao.petallink.AppError.Remote(
                resp.status.value, "create 未返回 200"
            )
        }
        return parseDriveFile(Json.parseToJsonElement(resp.bodyAsText()).jsonObject)
    }

    /** 更新文件（PATCH 覆盖） */
    suspend fun updateFile(fileId: String, name: String?): DriveFile {
        val meta = buildJsonObject {
            if (name != null) put("fileName", AsciiJson.escape(name))
        }
        val resp = client.executeWithRetry(
            HttpMethod.Patch, "$base/files/${Pkce.enc(fileId)}", HttpSemantics.WRITE,
        ) {
            contentType(ContentType.Application.Json)
            setBody(meta.toString())
        }
        require(resp.status.value == 200) {
            throw io.github.yuanbaobaao.petallink.AppError.Remote(
                resp.status.value, "update 未返回 200"
            )
        }
        return parseDriveFile(Json.parseToJsonElement(resp.bodyAsText()).jsonObject)
    }

    /** 软删除（PATCH recycled=true，踩坑：非物理删除） */
    suspend fun deleteFile(fileId: String) {
        val body = buildJsonObject { put("recycled", true) }
        val resp = client.executeWithRetry(
            HttpMethod.Patch, "$base/files/${Pkce.enc(fileId)}", HttpSemantics.WRITE,
        ) {
            contentType(ContentType.Application.Json)
            setBody(body.toString())
        }
        // 软删除成功合同：200 + File(id==请求 id, recycled==true)
        require(resp.status.value == 200) {
            throw io.github.yuanbaobaao.petallink.AppError.Remote(
                resp.status.value, "delete 未返回 200"
            )
        }
    }

    /**
     * 列出全部文件（自动翻页，1000 页上限）。
     */
    suspend fun listAllFiles(parentId: String?, maxPages: Int = 1000): List<DriveFile> {
        val all = mutableListOf<DriveFile>()
        var cursor: String? = null
        repeat(maxPages) {
            val (files, next) = listFiles(parentId, nextCursor = cursor)
            all += files
            cursor = next
            if (cursor == null) return all
        }
        throw io.github.yuanbaobaao.petallink.AppError.Remote(
            0, "listAllFiles 达到页数上限 $maxPages"
        )
    }

    /** 解析 DriveFile（容忍 String 类型的 size，踩坑 6） */
    private fun parseDriveFile(obj: JsonObject): DriveFile {
        return json.decodeFromJsonElement(DriveFile.serializer(), obj)
    }
}

