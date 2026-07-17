package io.github.yuanbaobaoo.petallink.sync.engine

import java.nio.file.Files
import java.nio.file.Paths
import java.nio.file.StandardCopyOption

/**
 * JVM POSIX rename（原子操作）
 */
actual fun platformRenameExpect(from: String, to: String) {
    Files.move(Paths.get(from), Paths.get(to), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
}

/**
 * JVM 删除文件
 */
actual fun platformDeleteExpect(path: String) {
    Files.deleteIfExists(Paths.get(path))
}
