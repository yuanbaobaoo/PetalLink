package io.github.yuanbaobaoo.petallink.ui

import io.github.yuanbaobaoo.petallink.ui.components.MateIcons
import io.github.yuanbaobaoo.petallink.ui.components.SvgIconCache
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * 矢量图标渲染 smoke 测试。
 *
 * 验证 32 个 SVG 资源全部存在，且能被 Skia SVGDOM 解析为非空 Picture。
 * 这保证图标系统在运行时不会因 SVG 文件缺失或 path 格式错误而整体失效。
 */
class MateIconTest {

    @Test
    fun `注册表包含 32 个图标 name`() {
        assertEquals(32, MateIcons.NAMES.size)
        // 抽查若干关键图标存在
        listOf("cloud", "folder", "folder-open", "search", "sync", "transfer", "settings").forEach { name ->
            assertTrue(name in MateIcons.NAMES, "缺少关键图标：$name")
        }
    }

    @Test
    fun `所有注册图标均可加载 SVG 字节并解析为 Picture`() {
        var failures = 0
        MateIcons.NAMES.forEach { name ->
            val picture = SvgIconCache.renderPicture(name, pixelSize = 24)
            if (picture == null) {
                failures++
                System.err.println("图标渲染失败：$name")
            } else {
                assertNotNull(picture, "图标 $name 解析返回了 null Picture")
            }
        }
        assertEquals(0, failures, "有 $failures 个图标渲染失败")
    }

    @Test
    fun `不同像素尺寸生成独立 Picture`() {
        val small = SvgIconCache.renderPicture("cloud", pixelSize = 16)
        val large = SvgIconCache.renderPicture("cloud", pixelSize = 48)
        assertNotNull(small)
        assertNotNull(large)
        // 不同尺寸应是不同 Picture 实例（缓存 key 不同）
        assertTrue(small !== large, "不同像素尺寸不应复用同一 Picture 实例")
    }

    @Test
    fun `缺失图标返回 null 而非抛异常`() {
        // 图标缺失应安全降级，绝不拖垮整页渲染
        assertNull(SvgIconCache.renderPicture("nonexistent-icon-xyz", pixelSize = 24))
    }

    @Test
    fun `非法像素尺寸返回 null`() {
        assertNull(SvgIconCache.renderPicture("cloud", pixelSize = 0))
        assertNull(SvgIconCache.renderPicture("cloud", pixelSize = -1))
    }
}
