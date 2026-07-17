package io.github.yuanbaobaoo.petallink.sync

/**
 * 冲突解决纯逻辑（对标 src/sync/conflict.rs）。
 *
 * 详见 docs/06 §冲突解决、docs/10 阶段 4 item 20。
 * 副本命名格式：{stem} ({side} {YYYY-MM-DD HH-mm-ss}){ext}
 * 时间戳取自败方。序号 0..1000。
 */
object ConflictResolver {

    /** 副本命名（对标 copy format） */
    fun copyName(
        originalName: String,
        side: ConflictSide,
        timestampFormatted: String,
        sequence: Int = 0,
    ): String {
        val (stem, ext) = splitNameExt(originalName)
        val sideLabel = side.label
        // 序号 > 0 时加后缀避免重名
        val seqSuffix = if (sequence > 0) " $sequence" else ""
        return "$stem ($sideLabel $timestampFormatted$seqSuffix)$ext"
    }

    /** 拆分文件名与扩展名 */
    private fun splitNameExt(name: String): Pair<String, String> {
        val dotIdx = name.lastIndexOf('.')
        return if (dotIdx > 0) {
            name.substring(0, dotIdx) to name.substring(dotIdx)
        } else {
            name to ""
        }
    }

    /** 冲突侧（败方） */
    enum class ConflictSide(val label: String) {
        LOCAL("本地"),
        CLOUD("云端"),
    }

    /** 副本序号上限（防无限重名） */
    const val MAX_SEQUENCE = 1000
}
