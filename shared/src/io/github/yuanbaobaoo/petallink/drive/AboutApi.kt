package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject

/**
 * 华为 About API（配额查询，对标 src/drive/about_api.rs）。
 *
 * 踩坑 6：配额字段是 String 类型，tolerant_parse_int 容忍 int/num/String。
 */
class AboutApi(
    private val client: DriveClient,
    private val base: String = DriveConstants.DRIVE_API_BASE,
) {
    /** 查询配额（fields=* 强制全字段） */
    suspend fun getQuota(): DriveQuota {
        val resp = client.executeWithRetry(
            HttpMethod.Get, "$base/about?fields=*", HttpSemantics.READ,
        )
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "about 未返回 200")
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        // storageQuota.userCapacity / usedSpace 可能是 String
        val sq = body["storageQuota"]?.jsonObject
        return DriveQuota(
            total = sq?.get("userCapacity"),
            used = sq?.get("usedSpace"),
            recycled = sq?.get("recycledSpace"),
        )
    }

    /**
     * 校验云盘剩余容量足以接收本次上传；配额缺失或不足时拒绝写入。
     */
    suspend fun ensureUploadCapacity(requiredBytes: Long) {
        require(requiredBytes >= 0L) { "上传大小不能为负数" }
        val quota = getQuota()
        val total = quota.totalBytes()
        val used = quota.usedBytes()
        if (total <= 0L || used < 0L || used > total) {
            throw AppError.Data("云盘配额响应无效，拒绝在容量未知时上传")
        }
        val available = total - used
        if (requiredBytes > available) {
            throw AppError.Data("云盘空间不足：需要 $requiredBytes 字节，剩余 $available 字节")
        }
    }

    private val Json = Json { ignoreUnknownKeys = true }
}
