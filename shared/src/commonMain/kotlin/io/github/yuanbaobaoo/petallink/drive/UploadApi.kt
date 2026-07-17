package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.auth.Pkce
import io.ktor.client.request.header
import io.ktor.client.request.setBody
import io.ktor.client.statement.*
import io.ktor.http.*
import io.ktor.http.content.ByteArrayContent
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray

/**
 * 华为上传 API（对标 src/drive/upload_api.rs）。
 *
 * 详见 docs/03 §上传、docs/10 阶段 2 item 10-11。
 * - 小文件（≤20MB）：multipart/related（踩坑 7，非 form-data）
 * - 大文件（>20MB）：断点续传，308 rangeList 连续性校验
 */
class UploadApi(
    private val client: DriveClient,
    private val base: String = "https://driveapis.cloud.huawei.com.cn/upload/drive/v1",
) {
    /**
     * 小文件 multipart/related 上传。
     *
     * @param fileName 文件名（multipart 元数据用原始 UTF-8，不 ascii 转义）
     * @param parentFolder 父文件夹 id（可空）
     * @param content 文件字节
     * @return 上传完成的 DriveFile
     */
    suspend fun uploadSmall(
        fileName: String,
        parentFolder: String?,
        content: ByteArray,
    ): DriveFile {
        // 踩坑 7：multipart/related，boundary = hwcloud_{timestamp_micros}
        val boundary = "hwcloud_${PlatformTime.micros()}"

        // 元数据 JSON（multipart 用原始 UTF-8，不 ascii 转义）
        val meta = buildJsonObject {
            put("fileName", fileName)
            if (!parentFolder.isNullOrBlank()) {
                putJsonArray("parentFolder") { add(JsonPrimitive(parentFolder)) }
            }
        }.toString()

        val resp = client.executeWithRetry(
            HttpMethod.Post, "$base/files?uploadType=multipart", HttpSemantics.WRITE,
        ) {
            // 手工拼 multipart/related body（Ktor 无原生 multipart/related，用 ByteArrayContent）
            val bodyBytes = buildMultipartRelatedBody(boundary, meta, content)
            setBody(
                ByteArrayContent(
                    bodyBytes,
                    ContentType.parse("multipart/related; boundary=$boundary"),
                ),
            )
        }
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "小文件上传未返回 200")
        val file = parseDriveFile(Json.parseToJsonElement(resp.bodyAsText()).jsonObject)
        // 完成校验：id 非空 + size 匹配 + name 匹配
        val verified = UploadProtocol.completeUploadFile(file, content.size.toLong(), fileName)
            ?: throw AppError.Remote(resp.status.value, "上传完成校验失败")
        if (!parentFolder.isNullOrBlank()) {
            require(DriveParsers.singleParent(verified, "upload") == parentFolder) {
                "上传响应父目录不一致"
            }
        }
        return verified
    }

    /**
     * 小文件覆盖更新；Update 绝不退化为 Create。
     */
    suspend fun uploadSmallUpdate(
        fileId: String,
        fileName: String,
        parentFolder: String?,
        content: ByteArray,
    ): DriveFile {
        require(fileId.isNotBlank()) { "Update 必须携带 fileId" }
        val boundary = "hwcloud_${PlatformTime.micros()}"
        val meta = buildJsonObject {
            put("fileName", fileName)
            if (!parentFolder.isNullOrBlank()) {
                putJsonArray("parentFolder") { add(JsonPrimitive(parentFolder)) }
            }
        }.toString()
        val resp = client.executeWithRetry(
            HttpMethod.Patch,
            "$base/files/${Pkce.enc(fileId)}?uploadType=multipart",
            HttpSemantics.WRITE,
        ) {
            setBody(
                ByteArrayContent(
                    buildMultipartRelatedBody(boundary, meta, content),
                    ContentType.parse("multipart/related; boundary=$boundary"),
                ),
            )
        }
        if (resp.status.value !in 200..299) {
            throw AppError.Remote(resp.status.value, "小文件 Update 失败，保留云端旧文件")
        }
        val file = parseDriveFile(Json.parseToJsonElement(resp.bodyAsText()).jsonObject)
        val verified = UploadProtocol.completeUploadFile(file, content.size.toLong(), fileName)
            ?: throw AppError.Remote(resp.status.value, "Update 完成校验失败")
        if (verified.id != fileId) throw AppError.Remote(resp.status.value, "Update 返回了不同 fileId")
        return verified
    }

    /**
     * 初始化断点续传会话（对标 init_resume_session）。
     * @return ResumeSession（含从 Location 头取的 session_url）
     */
    suspend fun initResume(
        fileName: String,
        parentFolder: String?,
        totalSize: Long,
    ): ResumeSession {
        val meta = buildJsonObject {
            put("fileName", fileName)
            if (!parentFolder.isNullOrBlank()) {
                putJsonArray("parentFolder") { add(JsonPrimitive(parentFolder)) }
            }
        }
        val resp = client.executeWithRetry(
            HttpMethod.Post, "$base/files?uploadType=resume", HttpSemantics.WRITE,
        ) {
            header("X-Upload-Content-Length", totalSize.toString())
            header(HttpHeaders.ContentType, ContentType.Application.Json.toString())
            setBody(meta.toString())
        }
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "断点续传初始化未返回 200")
        // 踩坑 12：session URL 从 Location 响应头取
        val sessionUrl = resp.headers[HttpHeaders.Location]
            ?: throw AppError.Remote(resp.status.value, "断点续传初始化缺少 Location 头")
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        val serverId = body["serverId"]?.jsonPrimitive?.contentOrNull
            ?: body["id"]?.jsonPrimitive?.contentOrNull
            ?: body["fileId"]?.jsonPrimitive?.contentOrNull
            ?: ""
        val uploadId = body["uploadId"]?.jsonPrimitive?.contentOrNull ?: ""
        val sliceSize = body["sliceSize"]?.jsonPrimitive?.content?.toLongOrNull() ?: 0L

        if (sessionUrl.isBlank()) throw AppError.Remote(resp.status.value, "断点续传初始化返回空 Location")
        return ResumeSession(
            serverId = serverId,
            uploadId = uploadId,
            sessionUrl = sessionUrl,
            chunkSize = UploadProtocol.validatedChunkSize(sliceSize),
        )
    }

    /**
     * 查询会话状态（PUT 零长度 Content-Range bytes 星号 斜杠 total，对标 query_session_status）。
     * @return (uploadedOffset, finalFile?)
     */
    suspend fun querySessionStatus(session: ResumeSession, totalSize: Long): SessionStatus {
        val url = session.requestUrl()
        val resp = client.executeWithRetry(
            HttpMethod.Put, url, HttpSemantics.READ,
        ) {
            header("Content-Range", "bytes */$totalSize")
            header(HttpHeaders.ContentLength, "0")
        }
        val status = resp.status.value
        if (status == 308) {
            // 解析 rangeList 确认偏移
            val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
            val rangeList = body["rangeList"]?.jsonArray?.map { it.jsonPrimitive.content }
                ?: throw AppError.Remote(308, "308 响应缺少 rangeList")
            val offset = UploadProtocol.parseConfirmedOffset(rangeList, totalSize)
            return SessionStatus(uploaded = offset, finalFile = null, sessionUrl = resp.headers[HttpHeaders.Location])
        }
        if (status in 200..299) {
            val file = parseDriveFile(Json.parseToJsonElement(resp.bodyAsText()).jsonObject)
            val verified = UploadProtocol.completeUploadFile(file, totalSize, null)
            return SessionStatus(
                uploaded = if (verified != null) totalSize else 0L,
                finalFile = verified,
                sessionUrl = resp.headers[HttpHeaders.Location],
            )
        }
        throw AppError.Remote(status, "会话状态查询失败")
    }

    /**
     * 提交一个 resume 分片。返回的 offset 只来自服务端 rangeList/完整 File；
     * 请求结果不确定时查询同一 session，绝不使用 offset + chunk.size 推算。
     */
    suspend fun putChunk(
        session: ResumeSession,
        offset: Long,
        totalSize: Long,
        content: ByteArray,
    ): ChunkResult {
        require(content.isNotEmpty()) { "上传分片不能为空" }
        val endExclusive = offset + content.size.toLong()
        require(offset >= 0 && endExclusive > offset && endExclusive <= totalSize) { "非法上传分片边界" }
        val end = endExclusive - 1L
        val response = try {
            client.executeWithRetry(
                HttpMethod.Put, session.requestUrl(), HttpSemantics.WRITE,
            ) {
                header(HttpHeaders.ContentRange, "bytes $offset-$end/$totalSize")
                header(HttpHeaders.ContentLength, content.size.toString())
                setBody(ByteArrayContent(content, ContentType.Application.OctetStream))
            }
        } catch (error: Throwable) {
            return reconcileUncertainChunk(session, offset, totalSize, error)
        }
        val rotatedUrl = response.headers[HttpHeaders.Location]
        val status = response.status.value
        if (status == 308) {
            val body = Json.parseToJsonElement(response.bodyAsText()).jsonObject
            val ranges = body["rangeList"]?.jsonArray?.map { it.jsonPrimitive.content }
                ?: throw AppError.Remote(308, "308 响应缺少 rangeList")
            return ChunkResult(
                uploaded = UploadProtocol.parseConfirmedOffset(ranges, totalSize),
                finalFile = null,
                sessionUrl = rotatedUrl,
            )
        }
        if (status in 200..299) {
            val body = Json.parseToJsonElement(response.bodyAsText()).jsonObject
            val file = runCatching { parseDriveFile(body) }.getOrNull()
            val verified = UploadProtocol.completeUploadFile(file, totalSize, null)
            if (verified != null) {
                return ChunkResult(totalSize, verified, rotatedUrl)
            }
            throw AppError.RemoteAmbiguous("分片返回 2xx，但没有可核验的完整 File")
        }
        if (status == 408 || status in 500..599) {
            return reconcileUncertainChunk(
                session, offset, totalSize,
                AppError.Remote(status, "分片写入结果不确定"),
            )
        }
        throw AppError.Remote(status, "上传分片失败")
    }

    /**
     * 分片结果不确定时查询同一会话以确认真实偏移，避免本地推算
     */
    private suspend fun reconcileUncertainChunk(
        session: ResumeSession,
        offset: Long,
        totalSize: Long,
        original: Throwable,
    ): ChunkResult {
        return try {
            val status = querySessionStatus(session, totalSize)
            ChunkResult(status.uploaded, status.finalFile, status.sessionUrl)
        } catch (queryError: Throwable) {
            throw AppError.RemoteAmbiguous("上传分片结果不确定且会话状态查询失败", queryError)
        }.also {
            if (it.finalFile == null && it.uploaded < offset) {
                throw AppError.Remote(0, "服务端确认偏移倒退")
            }
        }
    }

    /**
     * 构建 multipart/related body（返回真实字节流，对标原项目 upload_api/multipart.rs）。
     *
     * Body 布局（字节级精确）：
     *   --{boundary}\r\n
     *   Content-Type: application/json; charset=UTF-8\r\n\r\n
     *   {metadataJson}\r\n
     *   --{boundary}\r\n
     *   Content-Type: application/octet-stream\r\n\r\n
     *   {file bytes}\r\n
     *   --{boundary}--\r\n
     */
    private fun buildMultipartRelatedBody(
        boundary: String,
        metadataJson: String,
        content: ByteArray,
    ): ByteArray {
        val crlf = "\r\n".toByteArray()
        val doubleCrlf = "\r\n\r\n".toByteArray()
        val dashDash = "--".toByteArray()

        val boundaryBytes = boundary.toByteArray()
        val jsonBytes = metadataJson.toByteArray()
        val boundaryPrefix = dashDash + boundaryBytes + crlf
        val boundarySuffix = dashDash + boundaryBytes + dashDash + crlf

        // 组装所有部分
        val parts = mutableListOf<ByteArray>()

        // 第一部分：元数据
        parts.add(boundaryPrefix)
        parts.add("Content-Type: application/json; charset=UTF-8".toByteArray())
        parts.add(doubleCrlf)
        parts.add(jsonBytes)
        parts.add(crlf)

        // 第二部分：文件内容
        parts.add(boundaryPrefix)
        parts.add("Content-Type: application/octet-stream".toByteArray())
        parts.add(doubleCrlf)
        parts.add(content)
        parts.add(crlf)

        // 结束
        parts.add(boundarySuffix)

        return parts.fold(ByteArray(0)) { acc, ba -> acc + ba }
    }

    /**
     * 严格解析上传响应为 DriveFile，字段缺失时抛错
     */
    private fun parseDriveFile(obj: JsonObject): DriveFile =
        DriveParsers.parseDriveFileStrict(obj, "upload")

    private val Json = Json { ignoreUnknownKeys = true }
}

/**
 * 断点续传会话（对标 ResumeSession）
 */
data class ResumeSession(
    val serverId: String,
    val uploadId: String,
    val sessionUrl: String,
    val chunkSize: Long,
) {
    /**
     * 会话请求 URL（session_url 优先，踩坑 12）
     */
    fun requestUrl(
        uploadBase: String = "https://driveapis.cloud.huawei.com.cn/upload/drive/v1",
    ): String = when {
        sessionUrl.isNotBlank() -> sessionUrl
        serverId.isNotBlank() && uploadId.isNotBlank() ->
            "$uploadBase/files/$serverId?uploadId=$uploadId"
        else -> throw AppError.Remote(0, "无法确定会话请求 URL")
    }
}

/**
 * 会话状态查询结果
 */
data class SessionStatus(
    val uploaded: Long,
    val finalFile: DriveFile?,
    val sessionUrl: String? = null,
)

/**
 * 分片上传结果：服务端确认的已上传偏移、最终文件（上传完成时）及可能轮换的会话 URL
 */
data class ChunkResult(
    val uploaded: Long,
    val finalFile: DriveFile?,
    val sessionUrl: String? = null,
)
