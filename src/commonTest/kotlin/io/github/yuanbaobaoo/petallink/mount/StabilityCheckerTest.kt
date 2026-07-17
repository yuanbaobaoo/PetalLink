package io.github.yuanbaobaoo.petallink.mount

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * StabilityChecker 单测（对标 src/sync/stability.rs 三段式）。
 */
class StabilityCheckerTest {

    @Test
    fun mtime静止超过5秒判定稳定() {
        assertTrue(StabilityChecker.isMtimeStable(fileMtime = 100, nowSec = 106))
    }

    @Test
    fun mtime不足5秒判定不稳定() {
        assertFalse(StabilityChecker.isMtimeStable(fileMtime = 100, nowSec = 104))
    }

    @Test
    fun size稳定3秒判定通过() {
        assertTrue(StabilityChecker.isSizeStable(firstSampleSec = 100, nowSec = 103))
    }

    @Test
    fun size不足3秒不通过() {
        assertFalse(StabilityChecker.isSizeStable(firstSampleSec = 100, nowSec = 102))
    }

    @Test
    fun 编辑超过5分钟标记Editing() {
        assertTrue(StabilityChecker.isEditing(firstChangeSec = 0, nowSec = 301))
    }

    @Test
    fun 编辑不足5分钟不是Editing() {
        assertFalse(StabilityChecker.isEditing(firstChangeSec = 0, nowSec = 299))
    }

    @Test
    fun lsof空列表判定无占用() {
        assertTrue(StabilityChecker.isLsofClear(emptyList()))
    }

    @Test
    fun lsof仅白名单进程判定无占用() {
        assertTrue(StabilityChecker.isLsofClear(listOf("mds", "qlmanage")))
    }

    @Test
    fun lsof含非白名单进程判定有占用() {
        assertFalse(StabilityChecker.isLsofClear(listOf("Microsoft Word")))
    }

    @Test
    fun 三段式全过判定稳定() {
        assertTrue(StabilityChecker.isStable(
            fileMtime = 100, sizeFirstSampleSec = 100, nowSec = 106,
            processNames = listOf("mds"),
        ))
    }

    @Test
    fun 三段式mtime未过判定不稳定() {
        assertFalse(StabilityChecker.isStable(
            fileMtime = 104, sizeFirstSampleSec = 100, nowSec = 106,
            processNames = emptyList(),
        ))
    }

    @Test
    fun 三段式lsof未过判定不稳定() {
        assertFalse(StabilityChecker.isStable(
            fileMtime = 100, sizeFirstSampleSec = 100, nowSec = 106,
            processNames = listOf("Excel"),
        ))
    }
}
