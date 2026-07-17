package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.PlatformInode
import io.github.yuanbaobaoo.petallink.config.AppConfig
import java.nio.file.FileVisitResult
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.SimpleFileVisitor
import java.nio.file.attribute.BasicFileAttributes
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * JVM/macOS 递归扫描器；不跟随 symlink，所有路径统一过 [SkipFilter]。
 */
class JvmLocalFileScanner(
    mountRoot: Path,
    private val xattrs: XattrAccess,
    private val skipPatterns: List<String> = SkipFilter.DEFAULT_PATTERNS,
) : LocalFileScanner {
    private val root = mountRoot.toAbsolutePath().normalize()

    /**
     * 递归遍历挂载根，收集所有非跳过的文件与目录条目，按相对路径排序后返回。
     */
    override suspend fun scan(): List<LocalFileEntry> = withContext(Dispatchers.IO) {
        try {
            require(Files.isDirectory(root, LinkOption.NOFOLLOW_LINKS)) {
                "挂载路径不是可扫描目录: $root"
            }
            val result = mutableListOf<LocalFileEntry>()
            Files.walkFileTree(root, object : SimpleFileVisitor<Path>() {
                /**
                 * 访问目录前判断是否跳过；命中跳过规则则跳过整个子树，否则记录目录条目。
                 */
                override fun preVisitDirectory(dir: Path, attrs: BasicFileAttributes): FileVisitResult {
                    if (dir == root) return FileVisitResult.CONTINUE
                    if (shouldSkip(dir)) return FileVisitResult.SKIP_SUBTREE
                    result += entry(dir, attrs, isDirectory = true)
                    return FileVisitResult.CONTINUE
                }

                /**
                 * 访问普通文件时跳过符号链接与命中规则的文件，其余记录为文件条目。
                 */
                override fun visitFile(file: Path, attrs: BasicFileAttributes): FileVisitResult {
                    if (attrs.isSymbolicLink || !attrs.isRegularFile || shouldSkip(file)) {
                        return FileVisitResult.CONTINUE
                    }
                    result += entry(file, attrs, isDirectory = false)
                    return FileVisitResult.CONTINUE
                }
            })
            result.sortedBy(LocalFileEntry::relativePath)
        } catch (error: AppError) {
            throw error
        } catch (error: Throwable) {
            throw AppError.LocalIo("扫描本地目录失败: $root", error)
        }
    }

    /**
     * 按文件名和跳过模式判断该条目是否应被忽略。
     */
    private fun shouldSkip(path: Path): Boolean =
        SkipFilter.shouldSkip(path.fileName.toString(), skipPatterns)

    /**
     * 将单条路径转换为扫描结果条目，包含相对路径、大小、mtime、inode 及占位符状态。
     */
    private fun entry(path: Path, attrs: BasicFileAttributes, isDirectory: Boolean): LocalFileEntry {
        val normalized = path.toAbsolutePath().normalize()
        if (!normalized.startsWith(root)) throw AppError.LocalIo("扫描路径逸出挂载目录: $path")
        val relative = root.relativize(normalized).joinToString("/") { it.toString() }
        val state = if (isDirectory) null else readState(normalized)
        val size = if (isDirectory) 0L else attrs.size()
        return LocalFileEntry(
            absolutePath = normalized.toString(),
            relativePath = relative,
            inode = PlatformInode.readInode(normalized.toString()),
            size = size,
            mtime = attrs.lastModifiedTime().toMillis(),
            isDirectory = isDirectory,
            isPlaceholder = !isDirectory && size == 0L && state == PlaceholderState.PLACEHOLDER,
            placeholderState = state,
        )
    }

    /**
     * 读取并解析文件 state xattr，缺失返回 null，非法值抛出异常。
     */
    private fun readState(path: Path): PlaceholderState? {
        val raw = xattrs.get(path.toString(), AppConfig.XATTR_STATE) ?: return null
        val value = raw.decodeToString().trimEnd('\u0000')
        return PlaceholderState.fromXattr(value)
            ?: throw AppError.LocalIo("非法占位符 state xattr: $value ($path)")
    }
}
