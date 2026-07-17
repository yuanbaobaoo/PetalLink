package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.auth.Pkce
import io.ktor.client.request.header
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * 华为下载 API（对标 src/drive/download_api.rs）。
 *
 * 详见 docs/03 §下载、docs/10 阶段 2 item 11。
 * - GET /files/{id}?form=content
 * - Range 续传 + 206 Content-Range 校验
 * - 416 回退一次
 * - .tmp 临时文件 + POSIX rename 原子安装
 * - sha256 校验（1MB buffer）
 */
class DownloadApi(
    private val client: DriveClient,
    private val base: String = DriveConstants.DRIVE_API_BASE,
) {
    /**
     * 获取远端文件元数据（对标 fetch_remote_metadata）。
     * GET /files/{id}?fields=*，先读 ETag 响应头再读 body。
     */
    suspend fun fetchRemoteMetadata(fileId: String): RemoteMetadata {
        val resp = client.executeWithRetry(
            HttpMethod.Get, "$base/files/${Pkce.enc(fileId)}?fields=*", HttpSemantics.READ,
        )
        if (resp.status.value != 200) {
            throw AppError.Remote(resp.status.value, "fetchRemoteMetadata 未返回 200")
        }
        val headerEtag = resp.headers[HttpHeaders.ETag]
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        val id = body["id"]?.jsonPrimitive?.contentOrNull
            ?: throw AppError.Remote(0, "fetchRemoteMetadata 缺少 id")
        require(id == fileId) { "fetchRemoteMetadata id 不匹配: $id != $fileId" }
        // 踩坑 6：size 是 String
        val sizeStr = body["size"]?.jsonPrimitive?.contentOrNull
        val size = sizeStr?.toLongOrNull()
            ?: throw AppError.Remote(0, "fetchRemoteMetadata 缺少 size")
        val etag = headerEtag ?: body["etag"]?.jsonPrimitive?.contentOrNull
        val sha256 = body["sha256"]?.jsonPrimitive?.contentOrNull
            ?: body["fileSha256"]?.jsonPrimitive?.contentOrNull
        val editedTime = body["editedTime"]?.jsonPrimitive?.contentOrNull
            ?: body["edited_time"]?.jsonPrimitive?.contentOrNull
        return RemoteMetadata(
            fileId = id,
            size = size,
            etag = etag,
            sha256 = sha256,
            editedTime = editedTime,
        )
    }

    /**
     * 构建内容下载请求（对标 build_content_request）。
     * @return 响应（调用方负责流式写入 .tmp）
     */
    suspend fun buildContentRequest(
        fileId: String,
        offset: Long,
        etag: String?,
    ): HttpResponse = client.executeWithRetry(
        HttpMethod.Get,
        "$base/files/${Pkce.enc(fileId)}?form=content",
        HttpSemantics.READ,
    ) {
        if (offset > 0) header(HttpHeaders.Range, "bytes=$offset-")
        if (etag != null) header(HttpHeaders.IfMatch, etag)
    }

    private val Json = Json { ignoreUnknownKeys = true }
}

/** 远端文件元数据（稳定身份，对标 ResumeMetadata） */
data class RemoteMetadata(
    val fileId: String,
    val size: Long,
    val etag: String?,
    val sha256: String?,
    val editedTime: String? = null,
) {
    /** 是否有稳定身份（防版本混淆） */
    fun hasStableIdentity(): Boolean = etag != null || sha256 != null || editedTime != null

    fun resumeMetadata() = io.github.yuanbaobaoo.petallink.sync.engine.DownloadResumeMetadata(
        fileId = fileId,
        size = size,
        editedTime = editedTime,
        etag = etag,
        sha256 = sha256,
    )
}

/**
 * 解析 Content-Range 响应头（对标 parse_content_range）。
 * 格式：bytes start-end/total
 * @return (start, end, total) 或 null（格式非法）
 */
fun parseContentRange(header: String?): Triple<Long, Long, Long>? {
    if (header == null) return null
    val withoutPrefix = header.removePrefix("bytes ").trim()
    val slashIdx = withoutPrefix.indexOf('/')
    if (slashIdx < 0) return null
    val range = withoutPrefix.substring(0, slashIdx)
    val total = withoutPrefix.substring(slashIdx + 1).toLongOrNull() ?: return null
    val dashIdx = range.indexOf('-')
    if (dashIdx < 0) return null
    val start = range.substring(0, dashIdx).toLongOrNull() ?: return null
    val end = range.substring(dashIdx + 1).toLongOrNull() ?: return null
    return Triple(start, end, total)
}

/**
 * 校验 Range 响应偏移（对标 validated_response_offset）。
 * - 200 → 0（服务端忽略 Range，从头写）
 * - 206 → Content-Range 的 start
 * - 其他 → 抛异常
 */
fun validatedResponseOffset(
    status: Int,
    contentRangeHeader: String?,
    requestedOffset: Long,
    expectedTotal: Long,
): Long {
    if (status == 200) return 0L
    if (status == 206) {
        val (start, end, total) = parseContentRange(contentRangeHeader)
            ?: throw AppError.Remote(206, "Range 响应缺少 Content-Range")
        if (start != requestedOffset || total != expectedTotal || end < start || end >= total) {
            throw AppError.Remote(206, "Range 响应不匹配: start=$start end=$end total=$total")
        }
        return start
    }
    throw AppError.Remote(status, "不支持的成功状态码: $status")
}
