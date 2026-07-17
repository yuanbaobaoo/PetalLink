package io.github.yuanbaobaoo.petallink.drive

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

/**
 * 华为 Drive API 数据模型（对标原项目 src/drive/response.rs）
 *
 * 字段名严格对齐华为官方响应（驼峰），详见 docs/03。
 */

/**
 * 云端文件/文件夹
 */
@Serializable
data class DriveFile(
    val id: String? = null,
    val name: String? = null,
    val category: String? = null,
    val size: String? = null,
    @kotlinx.serialization.Transient val parent: String? = null,
    @SerialName("parentFolder") val parentFolder: List<String>? = parent?.let(::listOf),
    @SerialName("fileName") val fileName: String? = null,
    val description: String? = null,
    @SerialName("createdTime") val createdTime: String? = null,
    @SerialName("editedTime") val editedTime: String? = null,
    @SerialName("mimeType") val mimeType: String? = null,
    val contentHash: String? = null,
    val thumbnailLink: String? = null,
    val recycled: Boolean? = null,
    val digest: String? = contentHash,
    val etag: String? = null,
) {
    val modifiedTime: String? get() = editedTime
    val sizeBytes: Long get() = size?.toDoubleOrNull()?.takeIf { it.isFinite() && it >= 0 }?.toLong() ?: 0L
    val singleParentOrNull: String? get() = parentFolder?.singleOrNull() ?: parent
}

/**
 * 名称缺失时的兜底文案
 */
const val UNNAMED_FILE = "未命名"

/**
 * 文件展示名称：优先取 [DriveFile.name]，其次 [DriveFile.fileName]，最后兜底 [UNNAMED_FILE]。
 */
fun DriveFile.displayName(): String = name ?: fileName ?: UNNAMED_FILE

/**
 * 配额信息（size 字段华为可能返回 String，需容忍）
 */
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
fun DriveQuota.totalBytes(): Long = tolerantLong(total)

/**
 * 解析配额已用字节数，容忍 String/Int 形式，失败返回 0
 */
fun DriveQuota.usedBytes(): Long = tolerantLong(used)

/**
 * 将 JsonElement（可能为 String 或数字）安全转为 Long
 */
fun tolerantLong(el: JsonElement?): Long {
    if (el == null) return 0
    val s = el.toString().trim('"', ' ')
    return s.toLongOrNull() ?: s.toDoubleOrNull()?.takeIf { it.isFinite() }?.toLong() ?: 0L
}

/**
 * changes 增量事件中的一个条目
 */
@Serializable
data class ChangeEntry(
    val fileId: String? = null,
    val file: DriveFile? = null,
    val deleted: Boolean = false,
    @SerialName("trashDone") val trashDone: Boolean = false,
    @SerialName("recycled") val recycled: Boolean = false,
)

/**
 * changes 翻页响应
 */
@Serializable
data class ChangesResponse(
    val changes: List<ChangeEntry> = emptyList(),
    @SerialName("nextCursor") val nextCursor: String? = null,
    @SerialName("newStartCursor") val newStartCursor: String? = null,
)
