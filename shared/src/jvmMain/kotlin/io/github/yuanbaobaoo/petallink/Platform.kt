package io.github.yuanbaobaoo.petallink

import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Paths

/**
 * JVM 平台标识
 */
actual fun platformName(): String = "JVM (macOS)"

/**
 * JVM 平台下 [PlatformInode] 的 actual 实现：通过 NIO `unix:ino` 读取文件 inode。
 */
actual object PlatformInode {
    actual fun readInode(absolutePath: String): ULong {
        return try {
            val values = Files.readAttributes(
                Paths.get(absolutePath),
                "unix:ino",
                LinkOption.NOFOLLOW_LINKS,
            )
            val inode = values["ino"] as? Long
                ?: throw IllegalStateException("unix:ino 类型不是 Long")
            if (inode <= 0L) throw IllegalStateException("unix:ino 非正数: $inode")
            inode.toULong()
        } catch (e: Throwable) {
            throw AppError.LocalIo("读取 inode 失败: $absolutePath", e)
        }
    }
}
