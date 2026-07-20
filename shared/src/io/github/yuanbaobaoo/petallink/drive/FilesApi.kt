package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.auth.Pkce
import io.github.yuanbaobaoo.petallink.core.logging.Logger
import io.ktor.client.request.header
import io.ktor.client.request.setBody
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
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
    private val logger = Logger()

    /**
     * 列出文件（分页）
     */
    suspend fun listFiles(
        parentId: String?,
        pageSize: Int = 100,
        nextCursor: String? = null,
    ): Pair<List<DriveFile>, String?> {
        require(pageSize in 1..100) { "pageSize 必须在 1..100" }
        val folderToken = parentId?.takeIf { it.isNotBlank() } ?: "root"
        validateQueryLiteral(folderToken, "parentFolder")
        val query = "'$folderToken' in parentFolder"
        val params = mutableListOf(
            "fields=*",
            "pageSize=$pageSize",
            "queryParam=${query.encodeURLParameter()}",
        )
        nextCursor?.takeIf { it.isNotBlank() }?.let { params += "cursor=${it.encodeURLParameter()}" }
        val qs = params.joinToString("&")

        val resp = client.executeWithRetry(
            HttpMethod.Get, "$base/files?$qs", HttpSemantics.READ,
        )
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "list 未返回 200")
        val page = DriveParsers.parseFileListPage(Json.parseToJsonElement(resp.bodyAsText()), "list")
        return page.files to page.nextCursor
    }

    /**
     * 获取单个文件（fields=* 全字段）
     */
    suspend fun getFile(fileId: String): DriveFile {
        val resp = client.executeWithRetry(
            HttpMethod.Get,
            "$base/files/${Pkce.enc(fileId)}?fields=*",
            HttpSemantics.READ,
        )
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "get 未返回 200")
        return DriveParsers.parseDriveFileStrict(Json.parseToJsonElement(resp.bodyAsText()), "get")
    }

    /**
     * 创建文件/文件夹
     */
    suspend fun createFile(name: String, parentFolder: String?, isFolder: Boolean): DriveFile {
        require(name.isNotBlank()) { "名称不能为空" }
        val expectedParent = parentFolder ?: "root"
        if (isFolder) {
            findUniqueFolder(name, parentFolder, expectedParent)?.let {
                logger.info("drive.files_api.write") {
                    "创建文件夹前核验命中唯一同名目录，跳过 POST folderId=${it.id} folderName=$name parentId=$expectedParent"
                }
                return it
            }
        }
        return try {
            createFileOnce(name, parentFolder, isFolder, expectedParent)
        } catch (submitError: Throwable) {
            if (isFolder) {
                findUniqueFolder(name, parentFolder, expectedParent)?.let {
                    logger.info("drive.files_api.write") {
                        "创建文件夹响应不确定，父目录唯一核验确认已提交 folderId=${it.id} folderName=$name parentId=$expectedParent error=${submitError.message}"
                    }
                    return it
                }
            }
            throw submitError
        }
    }

    /**
     * 单次创建文件/文件夹，并严格校验响应名称、类型与父目录
     */
    private suspend fun createFileOnce(
        name: String,
        parentFolder: String?,
        isFolder: Boolean,
        expectedParent: String,
    ): DriveFile {
        val meta = buildJsonObject {
            put("fileName", name)
            if (parentFolder != null) putJsonArray("parentFolder") { add(parentFolder) }
            if (isFolder) put("mimeType", "application/vnd.huawei-apps.folder")
        }
        val resp = client.executeWithRetry(
            HttpMethod.Post, "$base/files", HttpSemantics.WRITE,
        ) {
            contentType(ContentType.Application.Json)
            setBody(AsciiJson.escapeNonAscii(meta.toString()))
        }
        // 踩坑 8：写操作必须 200（require_official_write_ok）
        require(resp.status.value == 200) {
            throw AppError.Remote(
                resp.status.value, "create 未返回 200"
            )
        }
        val file = DriveParsers.parseDriveFileStrict(Json.parseToJsonElement(resp.bodyAsText()), "create")
        require(file.name == name) { "create 响应名称不一致" }
        if (isFolder) require(DriveParsers.isFolderMime(file.mimeType)) { "create 响应不是文件夹" }
        require(DriveParsers.singleParent(file, "create") == expectedParent) { "create 响应父目录不一致" }
        return file
    }

    /**
     * 在指定父目录下查找同名唯一文件夹；存在多个时报歧义错误，用于创建去重
     */
    private suspend fun findUniqueFolder(
        name: String,
        parentFolder: String?,
        expectedParent: String,
    ): DriveFile? {
        val matches = listAllFiles(parentFolder).filter { file ->
            file.name == name && DriveParsers.isFolderMime(file.mimeType) &&
                runCatching { DriveParsers.singleParent(file) }.getOrNull() == expectedParent
        }
        if (matches.size > 1) throw AppError.Remote(0, "同一父目录存在多个同名文件夹，创建结果有歧义")
        return matches.singleOrNull()
    }

    /**
     * 更新文件（PATCH 覆盖）
     */
    suspend fun updateFile(fileId: String, name: String?): DriveFile {
        return try {
            updateFileOnce(fileId, name)
        } catch (submitError: Throwable) {
            val current = runCatching { getFile(fileId) }.getOrNull()
            if (current != null && current.id == fileId && (name == null || current.name == name)) current
            else throw submitError
        }
    }

    /**
     * 单次 PATCH 更新文件名，并严格校验响应 fileId 与名称
     */
    private suspend fun updateFileOnce(fileId: String, name: String?): DriveFile {
        val meta = buildJsonObject {
            if (name != null) put("fileName", name)
        }
        val resp = client.executeWithRetry(
            HttpMethod.Patch, "$base/files/${Pkce.enc(fileId)}", HttpSemantics.WRITE,
        ) {
            contentType(ContentType.Application.Json)
            setBody(AsciiJson.escapeNonAscii(meta.toString()))
        }
        require(resp.status.value == 200) {
            throw AppError.Remote(
                resp.status.value, "update 未返回 200"
            )
        }
        val file = DriveParsers.parseDriveFileStrict(Json.parseToJsonElement(resp.bodyAsText()), "update")
        require(file.id == fileId) { "update 响应 fileId 不一致" }
        if (name != null) require(file.name == name) { "update 响应名称不一致" }
        return file
    }

    /**
     * 软删除（PATCH recycled=true，踩坑：非物理删除）
     */
    suspend fun deleteFile(fileId: String) {
        try {
            deleteFileOnce(fileId)
        } catch (submitError: Throwable) {
            if (!verifyDeleted(fileId)) throw submitError
        }
    }

    /**
     * 单次软删除（recycled=true），并校验响应已确认删除
     */
    private suspend fun deleteFileOnce(fileId: String) {
        val body = buildJsonObject { put("recycled", true) }
        val resp = client.executeWithRetry(
            HttpMethod.Patch, "$base/files/${Pkce.enc(fileId)}", HttpSemantics.WRITE,
        ) {
            contentType(ContentType.Application.Json)
            setBody(body.toString())
        }
        require(resp.status.value == 200) {
            throw AppError.Remote(
                resp.status.value, "delete 未返回 200"
            )
        }
        val file = DriveParsers.parseDriveFileStrict(Json.parseToJsonElement(resp.bodyAsText()), "delete")
        require(file.id == fileId && file.recycled == true) { "delete 响应未确认 recycled=true" }
    }

    /**
     * 移动文件到新父目录；目标已是父目录则直接返回，否则尝试并在失败后核验
     */
    suspend fun moveFile(fileId: String, oldParent: String, newParent: String): DriveFile {
        validateQueryLiteral(oldParent, "removeParentFolder")
        validateQueryLiteral(newParent, "addParentFolder")
        val current = getFile(fileId)
        if (DriveParsers.singleParent(current, "move preflight") == newParent) return current
        return try {
            moveFileOnce(fileId, oldParent, newParent)
        } catch (submitError: Throwable) {
            val after = runCatching { getFile(fileId) }.getOrNull()
            if (after != null && after.id == fileId &&
                runCatching { DriveParsers.singleParent(after) }.getOrNull() == newParent
            ) after else throw submitError
        }
    }

    /**
     * 单次移动文件，通过 add/remove parentFolder 参数实现，并校验响应父目录
     */
    private suspend fun moveFileOnce(fileId: String, oldParent: String, newParent: String): DriveFile {
        val url = "$base/files/${Pkce.enc(fileId)}?fields=*" +
            "&addParentFolder=${newParent.encodeURLParameter()}" +
            "&removeParentFolder=${oldParent.encodeURLParameter()}"
        val resp = client.executeWithRetry(HttpMethod.Patch, url, HttpSemantics.WRITE) {
            contentType(ContentType.Application.Json)
            setBody("{}")
        }
        require(resp.status.value == 200) { "move 未返回 200" }
        val file = DriveParsers.parseDriveFileStrict(Json.parseToJsonElement(resp.bodyAsText()), "move")
        require(file.id == fileId) { "move 响应 fileId 不一致" }
        require(DriveParsers.singleParent(file, "move") == newParent) { "move 响应父目录不一致" }
        return file
    }

    /**
     * 核验文件是否已删除：404 视为已删除，200 时检查 recycled 标记
     */
    suspend fun verifyDeleted(fileId: String): Boolean {
        val resp = client.executeWithRetry(
            HttpMethod.Get,
            "$base/files/${Pkce.enc(fileId)}?fields=*",
            HttpSemantics.READ,
        )
        if (resp.status.value == 404) return true
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "删除结果核验失败")
        val file = DriveParsers.parseDriveFileStrict(Json.parseToJsonElement(resp.bodyAsText()), "verify delete")
        require(file.id == fileId) { "删除核验响应 fileId 不一致" }
        return file.recycled == true
    }

    /**
     * 按文件名关键词搜索，可选限定父目录，返回首页结果与下一页游标
     */
    suspend fun search(keyword: String, parentId: String?, pageSize: Int = 100): Pair<List<DriveFile>, String?> {
        require(pageSize in 1..100) { "pageSize 必须在 1..100" }
        validateQueryLiteral(keyword, "搜索关键词")
        var query = "fileName contains '$keyword'"
        parentId?.takeIf { it.isNotBlank() }?.let { parent ->
            validateQueryLiteral(parent, "parentFolder")
            query += " and '$parent' in parentFolder"
        }
        val url = "$base/files?fields=*&pageSize=$pageSize&queryParam=${query.encodeURLParameter()}"
        val resp = client.executeWithRetry(HttpMethod.Get, url, HttpSemantics.READ)
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "search 未返回 200")
        val page = DriveParsers.parseFileListPage(Json.parseToJsonElement(resp.bodyAsText()), "search")
        return page.files to page.nextCursor
    }

    /**
     * 列出全部文件（自动翻页，1000 页上限）。
     */
    suspend fun listAllFiles(parentId: String?, maxPages: Int = 1000): List<DriveFile> {
        val all = mutableListOf<DriveFile>()
        var cursor: String? = null
        val seen = mutableSetOf<String>()
        repeat(maxPages) {
            val (files, next) = listFiles(parentId, nextCursor = cursor)
            all += files
            cursor = next
            val nextCursor = cursor ?: return all
            if (!seen.add(nextCursor)) {
                throw AppError.Remote(0, "listAllFiles cursor 循环: $nextCursor")
            }
        }
        throw AppError.Remote(
            0, "listAllFiles 达到页数上限 $maxPages"
        )
    }

    /**
     * 校验查询字面量：禁止空串、单引号与反斜线，避免 query 注入
     */
    private fun validateQueryLiteral(value: String, field: String) {
        require(value.isNotBlank()) { "$field 不能为空" }
        require('\'' !in value && '\\' !in value) { "$field 不允许单引号或反斜线" }
    }
}
