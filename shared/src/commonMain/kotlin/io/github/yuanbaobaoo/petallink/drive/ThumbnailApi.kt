package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.auth.Pkce
import io.ktor.client.call.body
import io.ktor.client.request.header
import io.ktor.client.request.url
import io.ktor.client.statement.HttpResponse
import io.ktor.client.statement.bodyAsChannel
import io.ktor.client.statement.readRawBytes
import io.ktor.http.HttpHeaders

/**
 * 华为缩略图 API（对标 src/drive/thumbnail_api.rs）。
 *
 * GET /files/{id}/thumbnail?size=M，手工注入 Bearer（不走 executeWithRetry，
 * 因返回二进制流，且 401 重放由调用方处理）。
 * 详见 docs/10 阶段 2 item 13。
 */
class ThumbnailApi(
    private val client: DriveClient,
    private val base: String = DriveConstants.DRIVE_API_BASE,
) {
    /**
     * 获取缩略图二进制流。
     *
     * 端点：GET /thumbnails/{id}?form=content
     * 手工注入 Authorization Bearer 头（DriveClient.executeWithRetry 已处理 401 重放）。
     *
     * @param fileId 云端文件 ID
     * @param size 缩略图尺寸（S/M/L，默认 M）
     * @return 缩略图二进制字节；失败抛 [AppError]
     */
    suspend fun getThumbnail(fileId: String, size: String = "M"): ByteArray {
        val url = "$base/thumbnails/${Pkce.enc(fileId)}?form=content"
        val resp: HttpResponse = client.executeWithRetry(
            method = io.ktor.http.HttpMethod.Get,
            url = url,
            semantics = HttpSemantics.READ,
        )
        if (resp.status.value !in 200..299) {
            // 缩略图不存在或不可用（常见：非图片文件）→ 返回空
            if (resp.status.value == 404) return ByteArray(0)
            throw io.github.yuanbaobaoo.petallink.AppError.Remote(
                resp.status.value, "缩略图获取失败: ${resp.status.value}"
            )
        }
        // 读取二进制响应体
        return resp.readRawBytes()
    }
}
