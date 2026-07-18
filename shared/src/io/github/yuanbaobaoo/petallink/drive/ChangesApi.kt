package io.github.yuanbaobaoo.petallink.drive

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.auth.Pkce
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * 华为 Changes API（对标 src/drive/changes_api.rs）。
 *
 * 详见 docs/03 §changes、docs/10 阶段 2 item 12。
 * 关键踩坑：
 * - cursor 必传（13）
 * - nextCursor（中间页）vs newStartCursor（终页）绝不混用（15）
 * - 三种删除信号（14）
 * - 空中间页继续翻页
 */
class ChangesApi(
    private val client: DriveClient,
    private val base: String = DriveConstants.DRIVE_API_BASE,
) {
    /**
     * 获取初始 cursor
     */
    suspend fun getStartCursor(): String {
        val resp = client.executeWithRetry(
            HttpMethod.Get, "$base/changes/getStartCursor?fields=*", HttpSemantics.READ,
        )
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "getStartCursor 请求失败")
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        body["category"]?.let {
            if (it.jsonPrimitive.contentOrNull != "drive#startCursor") {
                throw AppError.Remote(0, "getStartCursor category 不是 drive#startCursor")
            }
        }
        val cursor = body["startCursor"]?.jsonPrimitive?.contentOrNull
            ?: throw AppError.Remote(0, "getStartCursor 缺少 startCursor")
        require(cursor.isNotBlank()) { "startCursor 为空" }
        return cursor
    }

    /**
     * 列出单页变更。
     * @return (changes, nextCursor?, newStartCursor?)
     */
    suspend fun listChanges(
        cursor: String,
        pageSize: Int = 100,
    ): Triple<List<DriveChange>, String?, String?> {
        require(cursor.isNotBlank()) { "changes cursor 必传非空（踩坑 13）" }
        val resp = client.executeWithRetry(
            HttpMethod.Get,
            "$base/changes?fields=*&pageSize=$pageSize&includeDeleted=true&cursor=${Pkce.enc(cursor)}",
            HttpSemantics.READ,
        )
        if (resp.status.value == 400 || resp.status.value == 410) {
            throw AppError.ChangesCursorInvalid(
                resp.status.value,
                "Changes cursor 已失效，必须保留旧 checkpoint 并全量重建",
            )
        }
        if (resp.status.value != 200) throw AppError.Remote(resp.status.value, "Changes:list 请求失败")
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        body["category"]?.let {
            if (it.jsonPrimitive.contentOrNull != "drive#changeList") {
                throw AppError.Remote(0, "Changes:list category 不是 drive#changeList")
            }
        }
        val rawChanges = body["changes"]?.runCatching { jsonArray }?.getOrNull()
            ?: throw AppError.Remote(0, "Changes:list 缺少 changes 数组")
        val changes = rawChanges.map { parseChange(it.jsonObject) }
        val nextCursor = parseCursor(body["nextCursor"], "nextCursor")
        val newStartCursor = parseCursor(body["newStartCursor"], "newStartCursor")
        return Triple(changes, nextCursor, newStartCursor)
    }

    /**
     * 列出全部变更（完整一轮追赶，对标 list_all_changes）。
     * @param startCursor 本轮起始 cursor
     * @return (全部变更, 终页 newStartCursor 作为下一轮起点)
     */
    suspend fun listAllChanges(startCursor: String, maxPages: Int = 10000): Pair<List<DriveChange>, String> {
        var cursor = startCursor
        val seen = mutableSetOf(cursor)
        val all = mutableListOf<DriveChange>()

        for (pageNum in 1..maxPages) {
            val (changes, nextCursor, newStartCursor) = listChanges(cursor)
            all += changes

            if (nextCursor != null) {
                // 中间页：继续翻页
                if (pageNum == maxPages) {
                    throw AppError.Remote(0, "达到页数上限 $maxPages 时仍有 nextCursor")
                }
                if (!seen.add(nextCursor)) {
                    throw AppError.Remote(0, "cursor 未推进或形成循环: $nextCursor")
                }
                cursor = nextCursor
                continue
            }

            // 终页：必须有非空 newStartCursor
            val finalCursor = newStartCursor
                ?: throw AppError.Remote(0, "终页缺少非空 newStartCursor")
            if (!ChangeParser.isCursorAdvanced(seen, finalCursor, cursor, changes.size)) {
                throw AppError.Remote(0, "newStartCursor 未推进或形成循环: $finalCursor")
            }
            return all to finalCursor
        }
        throw AppError.Remote(0, "未能在分页上限内结束")
    }

    /**
     * 解析单个 change（三种删除信号）
     */
    private fun parseChange(obj: JsonObject): DriveChange {
        obj["category"]?.let {
            if (it.jsonPrimitive.contentOrNull != "drive#change") {
                throw AppError.Remote(0, "change.category 不是 drive#change")
            }
        }
        obj["type"]?.let {
            if (it.jsonPrimitive.contentOrNull != "File") {
                throw AppError.Remote(0, "change.type 不是 File")
            }
        }
        val fileId = obj["fileId"]?.jsonPrimitive?.contentOrNull
            ?: throw AppError.Remote(0, "change 缺少 fileId")
        val deleted = obj["deleted"]?.jsonPrimitive?.booleanOrNull
            ?: throw AppError.Remote(0, "change 缺少 deleted 布尔字段")
        val changeType = obj["changeType"]?.jsonPrimitive?.contentOrNull
        val fileObj = obj["file"]?.takeIf { it !is JsonNull }?.jsonObject
        val recycled = fileObj?.get("recycled")?.jsonPrimitive?.booleanOrNull
        val removed = ChangeParser.isRemoved(deleted, changeType, recycled)
        if (fileObj != null) {
            val nestedId = fileObj["id"]?.jsonPrimitive?.contentOrNull
                ?: throw AppError.Remote(0, "change.file 缺少 id")
            if (nestedId != fileId) throw AppError.Remote(0, "change.file.id 与 fileId 不一致")
        }
        val file = if (removed) null else fileObj?.let {
            DriveParsers.parseDriveFileStrict(it, "change.file").also { parsed ->
                DriveParsers.singleParent(parsed, "change.file")
            }
        } ?: throw AppError.Remote(0, "非删除 change 缺少完整 file")

        return ChangeParser.parse(deleted, changeType, fileId, file, recycled)
    }

    private val Json = Json { ignoreUnknownKeys = true; isLenient = true }

    /**
     * 解析 cursor 字段：仅接受字符串或 null，空串归一化为 null，非法类型抛错
     */
    private fun parseCursor(value: kotlinx.serialization.json.JsonElement?, field: String): String? = when (value) {
        null, JsonNull -> null
        is JsonPrimitive -> if (value.isString) value.content.takeIf { it.isNotEmpty() }
            else throw AppError.Remote(0, "$field 必须是字符串或 null")
        else -> throw AppError.Remote(0, "$field 必须是字符串或 null")
    }
}
