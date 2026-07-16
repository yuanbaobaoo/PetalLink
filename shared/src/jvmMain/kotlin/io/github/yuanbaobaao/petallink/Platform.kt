package io.github.yuanbaobaao.petallink

import java.nio.file.Files
import java.nio.file.Paths
import java.nio.file.attribute.BasicFileAttributes

/** JVM 平台标识 */
actual fun platformName(): String = "JVM (macOS)"

actual object PlatformInode {
    actual fun readInode(absolutePath: String): ULong {
        val attrs = Files.readAttributes(Paths.get(absolutePath), BasicFileAttributes::class.java)
        // unix:ino 属性返回文件 inode
        return try {
            val inode = Files.getAttribute(Paths.get(absolutePath), "unix:ino") as Long
            inode.toULong()
        } catch (e: Throwable) {
            attrs.fileKey()?.hashCode()?.toLong()?.toULong() ?: 0UL
        }
    }
}
