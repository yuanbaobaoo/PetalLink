package io.github.yuanbaobaao.petallink.drive

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
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        // storageQuota.userCapacity / usedSpace 可能是 String
        val sq = body["storageQuota"]?.jsonObject
        return DriveQuota(
            total = sq?.get("userCapacity"),
            used = sq?.get("usedSpace"),
            recycled = sq?.get("recycledSpace"),
        )
    }

    private val Json = Json { ignoreUnknownKeys = true }
}
