package io.github.yuanbaobaao.petallink.drive

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

/**
 * 华为 Drive API 数据模型（对标原项目 src/drive/response.rs）
 *
 * 字段名严格对齐华为官方响应（驼峰），详见 docs/03。
 */

/** 云端文件/文件夹 */
@Serializable
data class DriveFile(
    val id: String? = null,
    val name: String? = null,
    val type: String? = null,           // "file" / "folder"
    val size: String? = null,           // 华为返回 String（容忍非数字）
    val parent: String? = null,
    @SerialName("file_name") val fileName: String? = null,
    @SerialName("created_time") val createdTime: String? = null,
    @SerialName("modified_time") val modifiedTime: String? = null,
    @SerialName("mime_type") val mimeType: String? = null,
    val digest: String? = null,         // 哈希摘要
    val etag: String? = null,
)

/** 配额信息（size 字段华为可能返回 String，需容忍） */
@Serializable
data class DriveQuota(
    val total: JsonElement? = null,     // 可能是 Int 也可能是 String
    val used: JsonElement? = null,
    @SerialName("recycled") val recycled: JsonElement? = null,
)

/**
 * 配额数值解析：容忍 String / Int 形式（docs/03 踩坑 18）。
 * 返回 Long 字节数；解析失败返回 0。
 */
fun DriveQuota.totalBytes(): Long = jsonElementToLong(total)
fun DriveQuota.usedBytes(): Long = jsonElementToLong(used)

/** 将 JsonElement（可能为 String 或数字）安全转为 Long */
private fun jsonElementToLong(el: JsonElement?): Long {
    if (el == null) return 0
    val s = el.toString().trim('"', ' ')
    return s.toLongOrNull() ?: 0L
}

/** changes 增量事件中的一个条目 */
@Serializable
data class ChangeEntry(
    val fileId: String? = null,
    val file: DriveFile? = null,
    val deleted: Boolean = false,
    @SerialName("trashDone") val trashDone: Boolean = false,
    @SerialName("recycled") val recycled: Boolean = false,
)

/** changes 翻页响应 */
@Serializable
data class ChangesResponse(
    val changes: List<ChangeEntry> = emptyList(),
    @SerialName("nextCursor") val nextCursor: String? = null,
    @SerialName("newStartCursor") val newStartCursor: String? = null,
)
