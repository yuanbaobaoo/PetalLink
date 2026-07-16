package io.github.yuanbaobaao.petallink.drive

import io.github.yuanbaobaao.petallink.AppError

/**
 * 上传协议纯逻辑（对标原项目 drive/upload_api/protocol.rs）
 *
 * 详见 docs/03 §上传、docs/10 阶段 2。
 */
object UploadProtocol {

    /** 小/大文件分界：20 MiB */
    const val SMALL_LARGE_THRESHOLD = 20L * 1024 * 1024
    /** 最小分块：256 KiB */
    const val MIN_CHUNK_SIZE = 256L * 1024
    /** 默认分块：2 MiB */
    const val DEFAULT_CHUNK_SIZE = 2L * 1024 * 1024
    /** 最大分块：64 MiB */
    const val MAX_CHUNK_SIZE = 64L * 1024 * 1024
    /** 单块本地重试次数（仅 Connect 类错误） */
    const val CHUNK_RETRIES = 3
    /** 最终状态查询最大轮询次数 */
    const val FINAL_STATUS_MAX_POLLS = 5

    /**
     * 校验分块大小：0 → 默认 2MiB；否则必须在 [MIN, MAX] 区间。
     */
    fun validatedChunkSize(chunkSize: Long): Long {
        val resolved = if (chunkSize == 0L) DEFAULT_CHUNK_SIZE else chunkSize
        require(resolved in MIN_CHUNK_SIZE..MAX_CHUNK_SIZE) {
            "chunkSize 必须在 $MIN_CHUNK_SIZE..$MAX_CHUNK_SIZE，当前 $resolved"
        }
        return resolved
    }

    /**
     * ★ 解析 308 响应确认的已上传偏移量（对标 parse_confirmed_offset）。
     *
     * 算法（docs/03 §308 rangeList 连续性校验）：
     * - rangeList 为数组，每个元素形如 "start-end"
     * - 必须从 0 开始，严格连续（start == 上一个 end+1），无间隙无重叠
     * - end 是包含的，必须 < totalSize
     * - 返回 last_end + 1（下一字节偏移）
     * - 空数组 → 0
     * - 缺少 rangeList / 格式非法 → 抛 [AppError.Remote]（remote_ambiguity）
     *
     * **绝不**回退到本地 offset + chunkLen 推算。
     */
    fun parseConfirmedOffset(rangeList: List<String>, totalSize: Long): Long {
        if (rangeList.isEmpty()) return 0L

        var expectedStart = 0L
        for (raw in rangeList) {
            // 拆 "start-end"
            val dashIdx = raw.indexOf('-')
            if (dashIdx < 0) {
                throw AppError.Remote(308, "非法上传范围: $raw")
            }
            val startStr = raw.substring(0, dashIdx)
            val endStr = raw.substring(dashIdx + 1)
            // 不允许第二个 '-'
            if (endStr.indexOf('-') >= 0) {
                throw AppError.Remote(308, "非法上传范围: $raw")
            }
            val start = startStr.toLongOrNull()
                ?: throw AppError.Remote(308, "非法上传范围起点: $raw")
            val end = endStr.toLongOrNull()
                ?: throw AppError.Remote(308, "非法上传范围终点: $raw")

            // 连续性 + 边界校验（全部必须满足）
            if (start != expectedStart || end < start || end >= totalSize) {
                throw AppError.Remote(
                    308,
                    "上传范围不连续或越界: $raw, 期望起点 $expectedStart, 总长度 $totalSize"
                )
            }
            expectedStart = end + 1L
        }
        return expectedStart
    }

    /**
     * 完成上传校验（对标 complete_upload_file）。
     * @return 校验通过的 DriveFile；id/size/name 不匹配返回 null
     */
    fun completeUploadFile(
        file: DriveFile?,
        expectedSize: Long,
        expectedName: String?,
    ): DriveFile? {
        if (file == null) return null
        val idOk = !file.id.isNullOrBlank()
        val sizeStr = file.size
        val size = sizeStr?.toLongOrNull()
        val sizeOk = size != null && size == expectedSize
        val nameOk = !file.name.isNullOrBlank() &&
            (expectedName == null || file.name == expectedName)
        return if (idOk && sizeOk && nameOk) file else null
    }
}
