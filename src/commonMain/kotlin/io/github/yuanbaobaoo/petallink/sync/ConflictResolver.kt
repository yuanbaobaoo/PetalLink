package io.github.yuanbaobaoo.petallink.sync

/**
 * 冲突解决纯逻辑（对标 src/sync/conflict.rs）。
 *
 * 详见 docs/06 §冲突解决、docs/10 阶段 4 item 20。
 * 副本命名格式：{stem} ({side} {YYYY-MM-DD HH-mm-ss}){ext}
 * 时间戳取自败方。序号 0..1000。
 */
object ConflictResolver {

    /**
     * 冲突判定结果：保留胜方，并以败方的修改时间生成副本名称。
     */
    data class Resolution(
        val winner: ConflictSide,
        val loser: ConflictSide,
        val loserTimestampMs: Long,
    )

    /**
     * 按修改时间决定冲突胜方；本地仅在比云端新超过 60 秒时胜出，其余情况由云端胜出。
     */
    fun resolve(localMtimeMs: Long, cloudEditedTimeMs: Long): Resolution =
        if (localMtimeMs - cloudEditedTimeMs > LOCAL_WIN_THRESHOLD_MS) {
            Resolution(ConflictSide.LOCAL, ConflictSide.CLOUD, cloudEditedTimeMs)
        } else {
            Resolution(ConflictSide.CLOUD, ConflictSide.LOCAL, localMtimeMs)
        }

    /**
     * 副本命名（对标 copy format）
     */
    fun copyName(
        originalName: String,
        side: ConflictSide,
        timestampFormatted: String,
        sequence: Int = 0,
    ): String {
        val (stem, ext) = splitNameExt(originalName)
        require(sequence in 0..MAX_SEQUENCE) { "冲突副本序号超出范围: $sequence" }
        val seqSuffix = if (sequence > 0) " ($sequence)" else ""
        return "$stem (${side.label} $timestampFormatted)$seqSuffix$ext"
    }

    /**
     * 拆分文件名与扩展名
     */
    private fun splitNameExt(name: String): Pair<String, String> {
        val dotIdx = name.lastIndexOf('.')
        return if (dotIdx > 0) {
            name.substring(0, dotIdx) to name.substring(dotIdx)
        } else {
            name to ""
        }
    }

    /**
     * 冲突侧（败方）
     */
    enum class ConflictSide(val label: String) {
        LOCAL("本地副本"),
        CLOUD("云端副本"),
    }

    /**
     * 副本序号上限（防无限重名）
     */
    const val MAX_SEQUENCE = 1000

    /**
     * 本地修改必须领先云端超过该阈值才胜出，用于吸收跨设备时钟误差。
     */
    const val LOCAL_WIN_THRESHOLD_MS = 60_000L
}
