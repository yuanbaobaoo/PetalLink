package io.github.yuanbaobaao.petallink.drive

/**
 * 增量变更事件（对标原项目 drive/changes_api.rs Change）
 *
 * 详见 docs/03 §changes、docs/10 阶段 2。
 */
data class DriveChange(
    val kind: ChangeKind,
    val fileId: String,
    val file: DriveFile?,
)

enum class ChangeKind { MODIFIED, REMOVED }

/**
 * 变更解析纯逻辑（对标原项目 Change::from_json）。
 *
 * 三种删除信号（docs/03 踩坑 14）：
 * 1. deleted == true
 * 2. changeType == "trashDone"
 * 3. file.recycled == true
 *
 * 任一信号 → Removed。不伪造墓碑（删除可能只带顶层 fileId）。
 */
object ChangeParser {

    /**
     * 判定单个 change 是否为删除（三种信号）。
     */
    fun isRemoved(deleted: Boolean, changeType: String?, recycled: Boolean?): Boolean {
        if (deleted) return true
        if (changeType == "trashDone") return true
        if (recycled == true) return true
        return false
    }

    /**
     * 解析单个 change（对标 Change::from_json，精简版）。
     *
     * @param deleted deleted 字段（必须存在，布尔）
     * @param changeType changeType 字段（可选）
     * @param fileId fileId 字段（必须非空）
     * @param file 解析后的 DriveFile（可选，含 recycled 字段需单独传入）
     * @param recycled file.recycled 字段（可选）
     */
    fun parse(
        deleted: Boolean,
        changeType: String?,
        fileId: String,
        file: DriveFile?,
        recycled: Boolean?,
    ): DriveChange {
        require(fileId.isNotBlank()) { "change 缺少 fileId" }
        val removed = isRemoved(deleted, changeType, recycled)
        val kind = if (removed) ChangeKind.REMOVED else ChangeKind.MODIFIED
        return DriveChange(kind = kind, fileId = fileId, file = if (removed) null else file)
    }

    /**
     * 校验终页游标推进（对标 list_all_changes 循环检测）。
     * @param seen 本轮已见游标集合
     * @param finalCursor 终页 newStartCursor
     * @param lastCursor 最后一页的 nextCursor（用于比较推进）
     * @return true 表示游标正常推进；false 表示未推进或循环
     */
    fun isCursorAdvanced(seen: Set<String>, finalCursor: String, lastCursor: String): Boolean {
        if (finalCursor.isBlank()) return false
        // 游标等于上一页且本轮有内容 → 未推进
        if (finalCursor == lastCursor) return false
        // 游标在本轮已见过 → 循环
        if (seen.contains(finalCursor)) return false
        return true
    }
}
