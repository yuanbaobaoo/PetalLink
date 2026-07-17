package io.github.yuanbaobaoo.petallink.config

import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/**
 * 应用全局常量（对标原项目 src/core/constants.rs + 各模块常量）
 *
 * 所有数值均为源码逐行核对后的精确实现值。改动前务必同步 docs/01~11。
 */
object AppConfig {

    // ------------------------------------------------------------------
    // 文件监听（docs/02 F-MOUNT-04 / docs/06 §3）
    // ------------------------------------------------------------------

    /**
     * 预热窗口：2 秒（非 8s，防历史回放误吞用户删除）
     */
    val WARMUP: Duration = 2.seconds

    /**
     * 去抖动窗口：3 秒（FSEvents 合并短时间内的连续修改）
     */
    val DEBOUNCE: Duration = 3.seconds

    // ------------------------------------------------------------------
    // 文件稳定性检查（docs/06 §3 三段式）
    // ------------------------------------------------------------------

    /**
     * mtime 静止宽限期：修改时间距现在 >5s 才认为写完
     */
    const val STABILITY_MTIME_GRACE_SECS = 5

    /**
     * 大小稳定持续时间：连续 3s 大小不变
     */
    const val STABILITY_SIZE_STABLE_SECS = 3

    /**
     * lsof 双重检查间隔：两次采样间隔 1s，两次都无占用才判定稳定
     */
    const val STABILITY_LSOF_DOUBLE_CHECK_SECS = 1

    /**
     * 持续编辑阈值：同一文件编辑时长 >5 分钟标记为 Editing
     */
    const val STABILITY_EDITING_THRESHOLD_SECS = 300

    /**
     * lsof 只读系统进程白名单（共 10 个），出现这些进程不算"被占用"
     */
    val STABILITY_LSOF_WHITELIST: List<String> = listOf(
        "mds", "mdworker_shared", "mdimport", "mdflagworker",
        "QuickLookSatellite", "qlmanage", "corespotlightd",
        "secd", "bird", "CoreServicesUIAgent",
    )

    // ------------------------------------------------------------------
    // 同步调度（docs/06 §8 CycleCoordinator）
    // ------------------------------------------------------------------

    /**
     * 最大并发传输数
     */
    const val MAX_CONCURRENT_TRANSFERS = 6

    /**
     * 增量 changes 轮询间隔：60 秒（Default 实现，非注释中的 900/10）
     */
    val POLL_INTERVAL: Duration = 60.seconds

    /**
     * 强制全量同步阈值：增量 changes 累计 >=300 条触发全量
     */
    const val INCREMENTAL_FORCED_FULL_THRESHOLD = 300

    // ------------------------------------------------------------------
    // 传输重试与退避（docs/06 §9 retry_policy.rs）
    // ------------------------------------------------------------------

    /**
     * 自动重试上限（超过则进入 Failed，需用户介入）
     */
    const val MAX_AUTOMATIC_ATTEMPTS = 5

    /**
     * 退避上限：2^attempt 秒，封顶 300s
     */
    val BACKOFF_CAP: Duration = 300.seconds

    // ------------------------------------------------------------------
    // 传输分块（docs/03 §上传/下载）
    // ------------------------------------------------------------------

    /**
     * 大小文件分界：>20MB 走分块上传/恢复
     */
    const val SMALL_LARGE_THRESHOLD_BYTES = 20L * 1024 * 1024

    /**
     * 分块大小：2MB（默认）
     */
    const val DEFAULT_CHUNK_SIZE_BYTES = 2L * 1024 * 1024

    /**
     * 单块重试次数
     */
    const val CHUNK_RETRIES = 3

    /**
     * 最终状态查询最大轮询次数（PUT bytes 全范围查询后）
     */
    const val FINAL_STATUS_MAX_POLLS = 5

    // ------------------------------------------------------------------
    // 数据持久化（docs/04 / docs/11）
    // ------------------------------------------------------------------

    /**
     * 数据库 schema 版本（inode 方案后 = 6）
     */
    const val SCHEMA_VERSION = 6

    /**
     * 传输历史保留条数（prune_transfer_history）
     */
    const val TRANSFER_HISTORY_RETENTION = 100

    // ------------------------------------------------------------------
    // xattr 键（inode 方案后仅 2 个，docs/11 §2.1）
    // ------------------------------------------------------------------

    /**
     * 占位符状态：唯一权威判据（placeholder / downloaded）
     */
    const val XATTR_STATE = "com.hwcloud.state"
    const val XATTR_FILE_ID = "com.hwcloud.fileId"

    /**
     * Finder 灰标：纯视觉反馈，buf[9] = 0x02（label index 7）
     */
    const val XATTR_FINDER_INFO = "com.apple.FinderInfo"

    // ------------------------------------------------------------------
    // 占位符 fileId 前缀（docs/04 §7，PENDING_FILE_ID_PREFIX）
    // ------------------------------------------------------------------

    /**
     * 尚未关联云端身份的占位 fileId 前缀
     */
    const val PENDING_FILE_ID_PREFIX = "pending:"

    // ------------------------------------------------------------------
    // 防振荡（docs/06 §12 recentlyDeletedPaths）
    // ------------------------------------------------------------------

    /**
     * 本地删除的防振荡保留时间：5 分钟内不把云端 DeleteFromCloud 再当新事件
     */
    val ANTI_OSCILLATION_TTL: Duration = (5 * 60).seconds
}
