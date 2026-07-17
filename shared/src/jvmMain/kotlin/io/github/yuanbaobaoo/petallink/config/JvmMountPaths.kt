package io.github.yuanbaobaoo.petallink.config

import java.nio.file.Path

/** JVM/macOS 挂载路径的唯一解析入口，避免 `~/` 被当作当前目录下的普通文件名。 */
object JvmMountPaths {
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
