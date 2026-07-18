package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.config.AppConfig
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.FileAlreadyExistsException
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * macOS JVM 占位符管理；state xattr 是唯一身份判据，FinderInfo 只负责视觉。
 */
class JvmPlaceholderManager(
    mountRoot: Path,
    private val xattrs: XattrAccess = MacXattrAccess,
    private val nowMs: () -> Long = System::currentTimeMillis,
) : PlaceholderManager {
    private val lexicalRoot = mountRoot.toAbsolutePath().normalize()
    private val root = requireSafeRoot(lexicalRoot)

    /**
     * 必要时为相对路径创建占位符；已存在文件则补齐 Finder 灰色标签后返回 false，新建成功返回 true。
     */
    override suspend fun createPlaceholderIfNeeded(relativePath: String): Boolean = io {
        val target = safeRelative(relativePath)
        if (Files.exists(target, LinkOption.NOFOLLOW_LINKS)) {
            rejectSymlink(target)
            if (!Files.isRegularFile(target, LinkOption.NOFOLLOW_LINKS)) return@io false
            val state = readState(target)
            if (state == PlaceholderState.PLACEHOLDER && Files.size(target) == 0L) {
                setFinderGreyLabelSync(target, true)
            }
            return@io false
        }
        ensureSafeParents(target.parent)
        createNewPlaceholder(target)
        true
    }

    /**
     * 强制为相对路径创建占位符；父路径不存在时逐级创建，文件已存在则抛出异常。
     */
    override suspend fun createPlaceholderStrict(relativePath: String) = io {
        val target = safeRelative(relativePath)
        ensureSafeParents(target.parent)
        createNewPlaceholder(target)
    }

    /**
     * 判断绝对路径是否为未下载的 0 字节占位符文件。
     */
    override suspend fun isPlaceholder(absolutePath: String): Boolean = io {
        val path = safeAbsolute(absolutePath)
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) return@io false
        rejectSymlink(path)
        Files.size(path) == 0L && readState(path) == PlaceholderState.PLACEHOLDER
    }

    /**
     * 将文件 state 标记为已下载并移除 Finder 灰色标签。
     */
    override suspend fun markDownloaded(absolutePath: String) = io {
        val path = safeAbsolute(absolutePath)
        rejectSymlink(path)
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("待标记 downloaded 的路径不是普通文件: $path")
        }
        xattrs.set(path.toString(), AppConfig.XATTR_STATE, PlaceholderState.DOWNLOADED.xattrValue.encodeToByteArray())
        setFinderGreyLabelSync(path, false)
    }

    /**
     * 设置或清除文件的 Finder 灰色标签（通过 FinderInfo 扩展属性）。
     */
    override suspend fun setFinderGreyLabel(absolutePath: String, on: Boolean) = io {
        setFinderGreyLabelSync(safeAbsolute(absolutePath), on)
    }

    /**
     * 删除本地文件或目录；目录按逆序删除，遇到用户写入的 0 字节文件会拒绝以防误删。
     */
    override suspend fun deleteLocal(absolutePath: String) = io {
        val path = safeAbsolute(absolutePath)
        if (!Files.exists(path, LinkOption.NOFOLLOW_LINKS)) return@io
        rejectSymlink(path)
        if (Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS)) {
            val paths = Files.walk(path).use { it.sorted(Comparator.reverseOrder()).toList() }
            for (candidate in paths) {
                rejectSymlink(candidate)
                if (Files.isRegularFile(candidate, LinkOption.NOFOLLOW_LINKS) &&
                    Files.size(candidate) == 0L && readState(candidate) != PlaceholderState.PLACEHOLDER
                ) {
                    throw AppError.LocalIo("拒绝删除含用户 0 字节文件的目录: $candidate")
                }
            }
            paths.forEach(Files::deleteIfExists)
            return@io
        }
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("拒绝删除非普通文件: $path")
        }
        if (Files.size(path) == 0L && readState(path) != PlaceholderState.PLACEHOLDER) return@io
        Files.deleteIfExists(path)
    }

    /**
     * 当占位符被本地修改后原子迁移到带时间戳的备份路径，清除其 state 标签并返回备份绝对路径；非占位符返回 null。
     */
    override suspend fun backupModifiedPlaceholder(absolutePath: String): String? = io {
        val source = safeAbsolute(absolutePath)
        if (!Files.isRegularFile(source, LinkOption.NOFOLLOW_LINKS)) return@io null
        rejectSymlink(source)
        if (Files.size(source) == 0L || readState(source) != PlaceholderState.PLACEHOLDER) return@io null

        val backup = uniqueBackupPath(source)
        moveAtomicallyWhenPossible(source, backup)
        try {
            xattrs.remove(backup.toString(), AppConfig.XATTR_STATE)
            setFinderGreyLabelSync(backup, false)
        } catch (error: Throwable) {
            runCatching { moveAtomicallyWhenPossible(backup, source) }
            throw AppError.LocalIo("备份已修改占位符时清理 xattr 失败: $backup", error)
        }
        backup.toString()
    }

    /**
     * 将任意普通本地文件原子移动到指定冲突副本路径，并清除占位符与 Finder 灰标。
     */
    suspend fun moveToConflictCopy(absoluteSource: String, absoluteTarget: String) = io {
        val source = safeAbsolute(absoluteSource)
        val target = safeAbsolute(absoluteTarget)
        rejectSymlink(source)
        if (!Files.isRegularFile(source, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("冲突源不是普通文件: $source")
        }
        if (Files.exists(target, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("冲突副本已存在: $target")
        }
        ensureSafeParents(target.parent)
        moveAtomicallyWhenPossible(source, target)
        try {
            xattrs.remove(target.toString(), AppConfig.XATTR_STATE)
            setFinderGreyLabelSync(target, false)
        } catch (error: Throwable) {
            runCatching { moveAtomicallyWhenPossible(target, source) }
            throw AppError.LocalIo("清理冲突副本标记失败: $target", error)
        }
    }

    /**
     * 下载失败时把冲突副本原子恢复到原路径；目标已被占用时拒绝覆盖。
     */
    suspend fun restoreConflictCopy(absoluteBackup: String, absoluteTarget: String) = io {
        val backup = safeAbsolute(absoluteBackup)
        val target = safeAbsolute(absoluteTarget)
        rejectSymlink(backup)
        if (Files.exists(target, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("冲突原路径已被占用，拒绝覆盖: $target")
        }
        moveAtomicallyWhenPossible(backup, target)
    }

    /**
     * 创建 0 字节占位符文件，写入 state xattr 并打上 Finder 灰色标签；中途失败会回滚。
     */
    private fun createNewPlaceholder(path: Path) {
        var created = false
        try {
            Files.createFile(path)
            created = true
            xattrs.set(path.toString(), AppConfig.XATTR_STATE, PlaceholderState.PLACEHOLDER.xattrValue.encodeToByteArray())
            setFinderGreyLabelSync(path, true)
        } catch (error: Throwable) {
            if (created) {
                runCatching { xattrs.remove(path.toString(), AppConfig.XATTR_STATE) }
                runCatching { xattrs.remove(path.toString(), AppConfig.XATTR_FINDER_INFO) }
                runCatching { Files.deleteIfExists(path) }
            }
            if (error is FileAlreadyExistsException) throw error
            throw AppError.LocalIo("创建占位符失败: $path", error)
        }
    }

    /**
     * 同步改写 FinderInfo 扩展属性以开启/关闭灰色标签；关闭后若 FinderInfo 全零则删除该属性。
     */
    private fun setFinderGreyLabelSync(path: Path, on: Boolean) {
        rejectSymlink(path)
        val current = xattrs.get(path.toString(), AppConfig.XATTR_FINDER_INFO)
        val finderInfo = when {
            current == null -> ByteArray(32)
            current.size < 32 -> current.copyOf(32)
            else -> current.copyOf()
        }
        finderInfo[9] = if (on) 0x02 else 0x00
        if (!on && finderInfo.all { it == 0.toByte() }) {
            xattrs.remove(path.toString(), AppConfig.XATTR_FINDER_INFO)
        } else {
            xattrs.set(path.toString(), AppConfig.XATTR_FINDER_INFO, finderInfo)
        }
    }

    /**
     * 读取并解析文件的 state xattr，缺失返回 null，非法值抛出异常。
     */
    private fun readState(path: Path): PlaceholderState? {
        val raw = xattrs.get(path.toString(), AppConfig.XATTR_STATE) ?: return null
        val text = raw.decodeToString().trimEnd('\u0000')
        return PlaceholderState.fromXattr(text)
            ?: throw AppError.LocalIo("非法占位符 state xattr: $text ($path)")
    }

    /**
     * 校验相对路径不含 `..`/`.` 且落在挂载根内，返回规范化后的绝对路径。
     */
    private fun safeRelative(relativePath: String): Path {
        require(relativePath.isNotBlank()) { "占位符相对路径不能为空" }
        val relative = Path.of(relativePath)
        require(!relative.isAbsolute && relative.none { it.toString() == ".." || it.toString() == "." }) {
            "非法占位符相对路径: $relativePath"
        }
        return safeAbsolute(root.resolve(relative).normalize().toString())
    }

    /**
     * 将绝对路径校验、规范化到挂载根之下，并拒绝路径中的符号链接组件。
     */
    private fun safeAbsolute(absolutePath: String): Path {
        val path = Path.of(absolutePath).toAbsolutePath().normalize()
        val acceptedRoot = when {
            path.startsWith(root) -> root
            path.startsWith(lexicalRoot) -> lexicalRoot
            else -> null
        }
        if (acceptedRoot == null || path == acceptedRoot) {
            throw AppError.LocalIo("路径不在挂载目录内: $absolutePath")
        }
        val relative = acceptedRoot.relativize(path)
        rejectSymlinkComponents(acceptedRoot, relative)
        return root.resolve(relative).normalize()
    }

    /**
     * 沿相对路径逐级解析，对存在的任一段执行符号链接校验。
     */
    private fun rejectSymlinkComponents(base: Path, relative: Path) {
        var current = base
        for (segment in relative) {
            current = current.resolve(segment)
            if (Files.exists(current, LinkOption.NOFOLLOW_LINKS)) rejectSymlink(current)
        }
    }

    /**
     * 逐级确保占位符的父目录存在、不是符号链接且确为目录，缺失则创建。
     */
    private fun ensureSafeParents(parent: Path) {
        val relative = root.relativize(parent)
        var current = root
        for (segment in relative) {
            current = current.resolve(segment)
            if (Files.exists(current, LinkOption.NOFOLLOW_LINKS)) {
                rejectSymlink(current)
                if (!Files.isDirectory(current, LinkOption.NOFOLLOW_LINKS)) {
                    throw AppError.LocalIo("占位符父路径不是目录: $current")
                }
            } else {
                Files.createDirectory(current)
            }
        }
    }

    /**
     * 对符号链接抛出异常，阻断操作。
     */
    private fun rejectSymlink(path: Path) {
        if (Files.isSymbolicLink(path)) throw AppError.LocalIo("拒绝操作符号链接: $path")
    }

    /**
     * 基于源文件名与当前时间戳生成同目录下不冲突的备份路径，必要时追加序号。
     */
    private fun uniqueBackupPath(source: Path): Path {
        val name = source.fileName.toString()
        val dot = name.lastIndexOf('.').takeIf { it > 0 } ?: name.length
        val stem = name.substring(0, dot)
        val extension = name.substring(dot)
        val base = "$stem.local-${nowMs()}"
        var candidate = source.resolveSibling("$base$extension")
        var sequence = 2
        while (Files.exists(candidate, LinkOption.NOFOLLOW_LINKS)) {
            candidate = source.resolveSibling("$base-$sequence$extension")
            sequence++
        }
        return candidate
    }

    /**
     * 优先尝试原子移动，不支持时回退到普通移动。
     */
    private fun moveAtomicallyWhenPossible(source: Path, target: Path) {
        try {
            Files.move(source, target, StandardCopyOption.ATOMIC_MOVE)
        } catch (_: AtomicMoveNotSupportedException) {
            Files.move(source, target)
        }
    }

    /**
     * 在 IO 调度器执行 [block]，捕获异常并统一包装为 [AppError.LocalIo]。
     */
    private suspend fun <T> io(block: () -> T): T = try {
        withContext(Dispatchers.IO) { block() }
    } catch (error: AppError) {
        throw error
    } catch (error: Throwable) {
        throw AppError.LocalIo(error.message ?: "本地文件操作失败", error)
    }

    /**
     * 校验挂载根真实存在、为目录且非符号链接，返回解析后的真实路径。
     */
    private fun requireSafeRoot(root: Path): Path {
        val normalized = root.toAbsolutePath().normalize()
        if (Files.isSymbolicLink(normalized) || !Files.isDirectory(normalized, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("挂载根目录不存在、不是目录或是符号链接: $normalized")
        }
        return normalized.toRealPath()
    }
}
