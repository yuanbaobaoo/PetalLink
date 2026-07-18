package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.config.AppConfig

/**
 * 文件稳定性检查纯逻辑（对标 src/sync/stability.rs 三段式）。
 *
 * 详见 docs/06 §稳定性检查、docs/10 阶段 3 item 21。
 * 三段式：
 * 1. mtime 静止 >5s
 * 2. size 稳定 3s
 * 3. lsof 双重检查（1s 间隔，白名单 10 个只读系统进程）
 *
 * 纯状态判定，IO 由调用方提供。
 */
object StabilityChecker {

    /**
     * 判定文件是否可视为"写完"（基于 mtime）。
     * @param fileMtime 文件修改时间（秒）
     * @param nowSec 当前时间（秒）
     * @return true 表示 mtime 已静止超过宽限期
     */
    fun isMtimeStable(fileMtime: Long, nowSec: Long): Boolean {
        return (nowSec - fileMtime) >= AppConfig.STABILITY_MTIME_GRACE_SECS
    }

    /**
     * 判定 size 是否已稳定（连续 3s 不变）。
     * @param firstSampleSec 第一次采样的时间
     * @param nowSec 当前时间
     * @return true 表示 size 已稳定 >=3s
     */
    fun isSizeStable(firstSampleSec: Long, nowSec: Long): Boolean {
        return (nowSec - firstSampleSec) >= AppConfig.STABILITY_SIZE_STABLE_SECS
    }

    /**
     * 判定是否为"持续编辑"状态（编辑时长 >5 分钟）。
     * @param firstChangeSec 首次变化时间
     * @param nowSec 当前时间
     */
    fun isEditing(firstChangeSec: Long, nowSec: Long): Boolean {
        return (nowSec - firstChangeSec) >= AppConfig.STABILITY_EDITING_THRESHOLD_SECS
    }

    /**
     * 判定 lsof 结果是否表示"无占用"（白名单进程不算占用）。
     * @param processNames 占用该文件的进程名列表
     * @return true 表示全部是白名单进程 → 无占用
     */
    fun isLsofClear(processNames: List<String>): Boolean {
        if (processNames.isEmpty()) return true
        return processNames.all { it in AppConfig.STABILITY_LSOF_WHITELIST }
    }

    /**
     * 综合稳定性判定（三段式全过）。
     * @param fileMtime 文件 mtime（秒）
     * @param sizeFirstSampleSec size 首次采样时间
     * @param nowSec 当前时间
     * @param processNames lsof 进程名列表
     */
    fun isStable(
        fileMtime: Long,
        sizeFirstSampleSec: Long,
        nowSec: Long,
        processNames: List<String>,
    ): Boolean {
        if (!isMtimeStable(fileMtime, nowSec)) return false
        if (!isSizeStable(sizeFirstSampleSec, nowSec)) return false
        if (!isLsofClear(processNames)) return false
        return true
    }
}
