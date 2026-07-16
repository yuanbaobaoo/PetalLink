package io.github.yuanbaobaao.petallink.mount

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * SkipFilter 单测（对标 src/mount/skip.rs）。
 */
class SkipFilterTest {

    @Test
    fun hwcloud前缀跳过() {
        assertTrue(SkipFilter.shouldSkip(".hwcloud_freeup-abc"))
        assertTrue(SkipFilter.shouldSkip(".hwcloud_placeholder"))
    }

    @Test
    fun tmp后缀跳过() {
        assertTrue(SkipFilter.shouldSkip("document.tmp"))
        assertTrue(SkipFilter.shouldSkip("download.tmp"))
    }

    @Test
    fun 旧版占位符跳过() {
        assertTrue(SkipFilter.shouldSkip(".hwcloud_placeholder"))
    }

    @Test
    fun 正常文件不跳过() {
        assertFalse(SkipFilter.shouldSkip("document.pdf"))
        assertFalse(SkipFilter.shouldSkip("照片.jpg"))
        assertFalse(SkipFilter.shouldSkip("folder"))
    }

    @Test
    fun 默认glob模式_匹配DS_Store() {
        assertTrue(SkipFilter.shouldSkip(".DS_Store"))
    }

    @Test
    fun 默认glob模式_匹配临时Office文件() {
        assertTrue(SkipFilter.shouldSkip("~\$document.docx"))
    }

    @Test
    fun 默认glob模式_匹配Trash() {
        assertTrue(SkipFilter.shouldSkip(".Trash"))
    }

    @Test
    fun globMatch_星号匹配() {
        assertTrue(SkipFilter.globMatch("*.tmp", "file.tmp"))
        assertFalse(SkipFilter.globMatch("*.tmp", "file.txt"))
    }

    @Test
    fun globMatch_问号匹配单字符() {
        assertTrue(SkipFilter.globMatch("?.txt", "a.txt"))
        assertFalse(SkipFilter.globMatch("?.txt", "ab.txt"))
    }

    @Test
    fun globMatch_字面量精确匹配() {
        assertTrue(SkipFilter.globMatch(".DS_Store", ".DS_Store"))
        assertFalse(SkipFilter.globMatch(".DS_Store", ".DS_Storex"))
    }

    @Test
    fun gitkeep不跳过() {
        // .gitkeep 是用户文件，受保护
        assertFalse(SkipFilter.shouldSkip(".gitkeep"))
    }
}
