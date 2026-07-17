package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

data class FileListPage(val files: List<DriveFile>, val nextCursor: String?)

object DriveParsers {
    private val timestampPattern = Regex(
        "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(?:\\.\\d+)?(?:Z|[+-]\\d{2}:\\d{2})$",
    )
    private val hashAliases = listOf("sha256", "md5", "md5Checksum", "fileSha256", "hash", "contentHash")

    fun parseFileListPage(element: JsonElement, context: String = "list"): FileListPage {
        val obj = element as? JsonObject ?: protocol(context, "响应顶层必须是对象")
        obj["category"]?.let {
            if ((it as? JsonPrimitive)?.contentOrNull != "drive#fileList") {
                protocol(context, "category 不是 drive#fileList")
            }
        }
        val rawFiles = obj["files"]?.takeIf { it !is JsonNull }?.let {
            runCatching { it.jsonArray }.getOrNull()
        } ?: protocol(context, "files 缺失或不是数组")
        val files = rawFiles.mapIndexed { index, value ->
            parseDriveFileStrict(value, "$context.files[$index]")
        }
        val cursor = when (val raw = obj["nextCursor"]) {
            null, JsonNull -> null
            is JsonPrimitive -> raw.contentOrNull?.takeIf { it.isNotEmpty() }
            else -> protocol(context, "nextCursor 必须是字符串、null 或缺失")
        }
        return FileListPage(files, cursor)
    }

    fun parseDriveFileStrict(element: JsonElement, context: String = "file"): DriveFile {
        val obj = element as? JsonObject ?: protocol(context, "File 必须是对象")
        val id = requiredString(obj, "id", context)
        val fileName = stringOrNull(obj["fileName"], "$context.fileName")
        val legacyName = stringOrNull(obj["name"], "$context.name")
        val name = (fileName ?: legacyName)?.takeIf { it.isNotEmpty() }
            ?: protocol(context, "fileName/name 缺失或为空")
        val mime = requiredString(obj, "mimeType", context)
        obj["category"]?.let {
            if (stringOrNull(it, "$context.category") != "drive#file") {
                protocol(context, "category 不是 drive#file")
            }
        }
        val size = parseOptionalNonnegativeNumber(obj["size"], "$context.size")
        val parents = when (val raw = obj["parentFolder"]) {
            null, JsonNull -> null
            else -> runCatching { raw.jsonArray }.getOrElse {
                protocol(context, "parentFolder 必须是字符串数组或 null")
            }.mapIndexed { index, parent ->
                stringOrNull(parent, "$context.parentFolder[$index]")?.takeIf(String::isNotEmpty)
                    ?: protocol(context, "parentFolder 元素必须是非空字符串")
            }
        }
        val created = optionalTimestamp(obj["createdTime"], "$context.createdTime")
        val edited = optionalTimestamp(obj["editedTime"], "$context.editedTime")
        val contentHash = hashAliases.firstNotNullOfOrNull { field ->
            stringOrNull(obj[field], "$context.$field")
        }
        val recycled = obj["recycled"]?.let { raw ->
            if (raw is JsonNull) null else raw.jsonPrimitive.booleanOrNull
                ?: protocol(context, "recycled 必须是布尔值或 null")
        }
        return DriveFile(
            id = id,
            name = name,
            category = obj["category"]?.jsonPrimitive?.contentOrNull,
            size = size?.toString(),
            parent = parents?.singleOrNull(),
            parentFolder = parents,
            fileName = fileName,
            description = stringOrNull(obj["description"], "$context.description"),
            createdTime = created,
            editedTime = edited,
            mimeType = mime,
            contentHash = contentHash,
            thumbnailLink = stringOrNull(obj["thumbnailLink"], "$context.thumbnailLink"),
            recycled = recycled,
            digest = contentHash,
            etag = stringOrNull(obj["etag"], "$context.etag"),
        )
    }

    fun singleParent(file: DriveFile, context: String = "file"): String =
        file.parentFolder?.singleOrNull()?.takeIf { it.isNotBlank() }
            ?: protocol(context, "当前只支持一个非空 parentFolder")

    fun isFolderMime(mimeType: String?): Boolean = mimeType?.lowercase() in setOf(
        "application/vnd.huawei-apps.folder",
        "application/vnd.huawei-app.folder",
        "application/vnd.google-apps.folder",
        "application/x-folder",
    )

    private fun requiredString(obj: JsonObject, field: String, context: String): String =
        stringOrNull(obj[field], "$context.$field")?.takeIf { it.isNotEmpty() }
            ?: protocol(context, "$field 缺失、类型错误或为空")

    private fun stringOrNull(value: JsonElement?, context: String): String? = when (value) {
        null, JsonNull -> null
        is JsonPrimitive -> if (value.isString) value.content else protocol(context, "必须是字符串或 null")
        else -> protocol(context, "必须是字符串或 null")
    }

    private fun optionalTimestamp(value: JsonElement?, context: String): String? {
        val timestamp = stringOrNull(value, context) ?: return null
        if (!timestampPattern.matches(timestamp)) protocol(context, "必须是 RFC3339 时间")
        return timestamp
    }

    private fun parseOptionalNonnegativeNumber(value: JsonElement?, context: String): Long? {
        if (value == null || value is JsonNull) return null
        val primitive = value as? JsonPrimitive ?: protocol(context, "必须是非负数字、字符串或 null")
        val number = primitive.content.toDoubleOrNull()
            ?.takeIf { it.isFinite() && it >= 0 }
            ?: protocol(context, "必须是非负数字、字符串或 null")
        return number.toLong()
    }

    private fun protocol(context: String, cause: String): Nothing =
        throw AppError.Remote(0, "$context: $cause")
}
