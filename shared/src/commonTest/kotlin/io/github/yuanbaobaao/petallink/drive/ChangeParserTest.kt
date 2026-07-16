package io.github.yuanbaobaao.petallink.drive

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * ChangeParser 单测（对标 docs/03 §三种删除信号）。
 */
class ChangeParserTest {

    @Test
    fun deleted为true判定为Removed() {
        val change = ChangeParser.parse(
            deleted = true, changeType = null, fileId = "f1", file = null, recycled = null,
        )
        assertEquals(ChangeKind.REMOVED, change.kind)
        assertNull(change.file)
    }

    @Test
    fun changeType为trashDone判定为Removed() {
        // 踩坑 14：trashDone 也是删除信号
        val change = ChangeParser.parse(
            deleted = false, changeType = "trashDone", fileId = "f1", file = null, recycled = null,
        )
        assertEquals(ChangeKind.REMOVED, change.kind)
    }

    @Test
    fun recycled为true判定为Removed() {
        // 踩坑 14：file.recycled=true 也是删除信号
        val change = ChangeParser.parse(
            deleted = false, changeType = null, fileId = "f1", file = null, recycled = true,
        )
        assertEquals(ChangeKind.REMOVED, change.kind)
    }

    @Test
    fun 三种信号都为假判定为Modified() {
        val file = DriveFile(id = "f1", name = "a.txt", size = "10")
        val change = ChangeParser.parse(
            deleted = false, changeType = "fileEdit", fileId = "f1", file = file, recycled = false,
        )
        assertEquals(ChangeKind.MODIFIED, change.kind)
        assertEquals(file, change.file)
    }

    @Test
    fun isCursorAdvanced_正常推进返回true() {
        assertTrue(ChangeParser.isCursorAdvanced(setOf("c1", "c2"), "c3", "c2"))
    }

    @Test
    fun isCursorAdvanced_等于上一页返回false() {
        assertFalse(ChangeParser.isCursorAdvanced(setOf("c1"), "c1", "c1"))
    }

    @Test
    fun isCursorAdvanced_循环返回false() {
        assertFalse(ChangeParser.isCursorAdvanced(setOf("c1", "c2"), "c1", "c2"))
    }

    @Test
    fun isCursorAdvanced_空游标返回false() {
        assertFalse(ChangeParser.isCursorAdvanced(emptySet(), "", "c1"))
    }
}
