package io.github.yuanbaobaao.petallink.mount

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.PlatformInode
import io.github.yuanbaobaao.petallink.config.AppConfig
import java.nio.file.FileVisitResult
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.SimpleFileVisitor
import java.nio.file.attribute.BasicFileAttributes
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** JVM/macOS 递归扫描器；不跟随 symlink，所有路径统一过 [SkipFilter]。 */
class JvmLocalFileScanner(
    mountRoot: Path,
    private val xattrs: XattrAccess,
    private val skipPatterns: List<String> = SkipFilter.DEFAULT_PATTERNS,
) : LocalFileScanner {
    private val root = mountRoot.toAbsolutePath().normalize()

    override suspend fun scan(): List<LocalFileEntry> = withContext(Dispatchers.IO) {
        try {
            require(Files.isDirectory(root, LinkOption.NOFOLLOW_LINKS)) {
                "挂载路径不是可扫描目录: $root"
            }
            val result = mutableListOf<LocalFileEntry>()
            Files.walkFileTree(root, object : SimpleFileVisitor<Path>() {
                override fun preVisitDirectory(dir: Path, attrs: BasicFileAttributes): FileVisitResult {
                    if (dir == root) return FileVisitResult.CONTINUE
                    if (shouldSkip(dir)) return FileVisitResult.SKIP_SUBTREE
                    result += entry(dir, attrs, isDirectory = true)
                    return FileVisitResult.CONTINUE
                }

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

    private fun shouldSkip(path: Path): Boolean =
        SkipFilter.shouldSkip(path.fileName.toString(), skipPatterns)

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

    private fun readState(path: Path): PlaceholderState? {
        val raw = xattrs.get(path.toString(), AppConfig.XATTR_STATE) ?: return null
        val value = raw.decodeToString().trimEnd('\u0000')
        return PlaceholderState.fromXattr(value)
            ?: throw AppError.LocalIo("非法占位符 state xattr: $value ($path)")
    }
}
