package io.github.yuanbaobaoo.petallink.config

import java.nio.file.Path

/**
 * JVM/macOS 挂载路径的唯一解析入口，避免 `~/` 被当作当前目录下的普通文件名。
 */
object JvmMountPaths {
    /**
     * 解析原始路径字符串，正确展开 `~` / `~/` 为用户主目录并返回规范化的绝对路径。
     */
    fun resolve(raw: String): Path {
        val value = raw.trim()
        val home = Path.of(System.getProperty("user.home")).toAbsolutePath().normalize()
        return when {
            value == "~" -> home
            value.startsWith("~/") -> home.resolve(value.removePrefix("~/"))
            else -> Path.of(value).toAbsolutePath().normalize()
        }.normalize()
    }
}
