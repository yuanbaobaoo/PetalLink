package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.config.AppConfig
import io.github.yuanbaobaoo.petallink.config.ConfigStore
import io.github.yuanbaobaoo.petallink.config.JvmMountPaths
import io.github.yuanbaobaoo.petallink.data.PetalLinkDb
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.data.TransferTask
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.mount.MacXattrAccess
import io.github.yuanbaobaoo.petallink.mount.XattrAccess
import io.github.yuanbaobaoo.petallink.sync.SyncStatus
import io.github.yuanbaobaoo.petallink.sync.TransferState
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.time.Instant

/** 直接云端写入后的本地路径、DB 基线与删除留痕结算。 */
internal class JvmDriveMutationSettler(
    private val configStore: ConfigStore,
    private val db: PetalLinkDb,
    private val xattrs: XattrAccess = MacXattrAccess,
) {
    data class PathChangePlan(
        val fileId: String,
        val oldRoot: String,
        val newRoot: String,
        val affected: List<SyncItem>,
        val mountRoot: Path,
    )

    data class DeletePlan(
        val fileId: String,
        val rootItem: SyncItem?,
        val affected: List<SyncItem>,
        val mountRoot: Path?,
    )

    suspend fun planRename(fileId: String, newName: String): PathChangePlan? {
        validateSegment(newName)
        val item = db.syncItems.findByFileId(fileId) ?: run {
            ensureNoActiveTransfer(fileId, null)
            return null
        }
        val parent = item.localPath.substringBeforeLast('/', "")
        val newRoot = if (parent.isEmpty()) newName else "$parent/$newName"
        return planPathChange(item, newRoot)
    }

    suspend fun planMove(fileId: String, newParentFolder: String): PathChangePlan? {
        val item = db.syncItems.findByFileId(fileId) ?: run {
            ensureNoActiveTransfer(fileId, null)
            return null
        }
        val parentPath = when (newParentFolder) {
            "root" -> ""
            else -> db.syncItems.findByFileId(newParentFolder)
                ?.takeIf(SyncItem::isFolder)?.localPath
                ?: throw AppError.Data("无法解析目标云端目录的本地路径")
        }
        val name = item.localPath.substringAfterLast('/')
        val newRoot = if (parentPath.isEmpty()) name else "$parentPath/$name"
        return planPathChange(item, newRoot)
    }

    private suspend fun planPathChange(item: SyncItem, newRoot: String): PathChangePlan {
        validateRelative(newRoot)
        val oldRoot = item.localPath
        require(newRoot != oldRoot) { "目标路径与当前路径相同" }
        require(!newRoot.startsWith("$oldRoot/")) { "拒绝把目录移动到自身子树" }
        ensureNoActiveTransfer(item.fileId, oldRoot)
        ensureNoActiveTransfer(null, newRoot)
        val all = db.syncItems.selectAll()
        val affected = all.filter { inSubtree(it.localPath, oldRoot) }
        check(affected.isNotEmpty()) { "路径变更源基线不存在" }
        if (all.any { !inSubtree(it.localPath, oldRoot) && inSubtree(it.localPath, newRoot) }) {
            throw AppError.Data("目标同步基线已被其他文件或目录占用")
        }
        val root = configuredRoot()
        val oldPath = safePath(root, oldRoot)
        val newPath = safePath(root, newRoot)
        if (Files.exists(newPath, LinkOption.NOFOLLOW_LINKS) && oldPath != newPath) {
            throw AppError.LocalIo("目标本地路径已存在，拒绝先修改云端")
        }
        if (Files.exists(oldPath, LinkOption.NOFOLLOW_LINKS)) {
            rejectUnsafeType(oldPath, item)
            xattrs.set(oldPath.toString(), AppConfig.XATTR_FILE_ID, item.fileId.encodeToByteArray())
        }
        return PathChangePlan(item.fileId, oldRoot, newRoot, affected, root)
    }

    suspend fun settlePathChange(plan: PathChangePlan, verified: DriveFile) {
        val oldPath = safePath(plan.mountRoot, plan.oldRoot)
        val newPath = safePath(plan.mountRoot, plan.newRoot)
        if (Files.exists(oldPath, LinkOption.NOFOLLOW_LINKS)) {
            rejectUnsafeType(oldPath, plan.affected.first { it.fileId == plan.fileId })
            if (Files.exists(newPath, LinkOption.NOFOLLOW_LINKS)) {
                throw AppError.LocalIo("远端已变更，但目标本地路径已存在，已保留内容")
            }
            requireSafeParent(plan.mountRoot, newPath.parent)
            moveNoReplace(oldPath, newPath)
        } else if (Files.exists(newPath, LinkOption.NOFOLLOW_LINKS)) {
            val targetId = xattrs.get(newPath.toString(), AppConfig.XATTR_FILE_ID)?.decodeToString()?.trimEnd('\u0000')
            if (targetId != plan.fileId) throw AppError.LocalIo("目标路径无法证明是同一云端文件")
        }
        val replacements = plan.affected.map { item ->
            val suffix = item.localPath.removePrefix(plan.oldRoot)
            val path = plan.newRoot + suffix
            if (item.fileId == plan.fileId) item.copy(
                localPath = path,
                name = verified.name ?: path.substringAfterLast('/'),
                parentFolderId = verified.parentFolder?.singleOrNull(),
                cloudEditedTime = verified.editedTime?.let(::parseTime),
            ) else item.copy(localPath = path)
        }
        db.syncItems.replaceSubtree(plan.oldRoot, replacements)
    }

    suspend fun planDelete(fileId: String): DeletePlan {
        val item = db.syncItems.findByFileId(fileId)
        ensureNoActiveTransfer(fileId, item?.localPath)
        if (item == null) return DeletePlan(fileId, null, emptyList(), null)
        val root = configuredRoot()
        val affected = db.syncItems.selectAll().filter { inSubtree(it.localPath, item.localPath) }
        verifyDeleteSnapshot(root, item.localPath, affected)
        return DeletePlan(fileId, item, affected, root)
    }

    suspend fun settleDelete(plan: DeletePlan, fallbackName: String?) {
        if (plan.rootItem != null && plan.mountRoot != null) {
            verifyDeleteSnapshot(plan.mountRoot, plan.rootItem.localPath, plan.affected)
            val local = safePath(plan.mountRoot, plan.rootItem.localPath)
            if (Files.exists(local, LinkOption.NOFOLLOW_LINKS)) {
                Files.walk(local).use { stream ->
                    stream.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists)
                }
            }
            db.syncItems.updateSubtreeStatus(plan.rootItem.localPath, SyncStatus.DELETED, null)
        }
        val now = System.currentTimeMillis()
        try {
            db.transfers.insert(TransferTask(
                id = null,
                direction = TransferDirection.DELETE,
                fileId = plan.fileId,
                localPath = null,
                name = plan.rootItem?.name ?: fallbackName ?: plan.fileId,
                state = TransferState.Completed,
                errorMessage = null,
                createdAt = now,
                finishedAt = now,
                relativePath = plan.rootItem?.localPath,
                operation = 4,
            ))
        } catch (error: Throwable) {
            throw AppError.Data("${DELETE_TRACE_ERROR_PREFIX}文件已删除，但传输记录写入失败：${error.message}")
        }
        try {
            db.transfers.pruneHistory(100)
        } catch (_: Throwable) {
            // 修剪历史不得把“远端已删 + 留痕已写”伪装成删除失败。
        }
    }

    private suspend fun ensureNoActiveTransfer(fileId: String?, path: String?) {
        val terminal = setOf(TransferState.Completed, TransferState.Failed, TransferState.Canceled)
        val active = db.transfers.selectAll().any { task ->
            task.state !in terminal &&
                (fileId != null && task.fileId == fileId || path != null && task.relativePath?.let { pathsOverlap(it, path) } == true)
        }
        if (active) throw AppError.Data("该文件存在活动或待恢复任务，请稍后重试")
    }

    private fun verifyDeleteSnapshot(root: Path, rootRelative: String, affected: List<SyncItem>) {
        val absolute = safePath(root, rootRelative)
        if (!Files.exists(absolute, LinkOption.NOFOLLOW_LINKS)) return
        val byPath = affected.associateBy(SyncItem::localPath)
        if (byPath.size != affected.size) throw AppError.Data("同步基线存在重复路径")
        Files.walk(absolute).use { stream ->
            stream.forEach { path ->
                if (Files.isSymbolicLink(path)) throw AppError.LocalIo("拒绝删除含符号链接的目录: $path")
                val relative = root.relativize(path).joinToString("/")
                val item = byPath[relative] ?: throw AppError.LocalIo("本地子树含未纳入基线的内容，已拒绝删除: $relative")
                rejectUnsafeType(path, item)
                if (!item.isFolder) {
                    item.localSize?.let { if (Files.size(path) != it) throw AppError.LocalIo("本地文件大小已变化: $relative") }
                    item.localMtime?.let {
                        if (Files.getLastModifiedTime(path, LinkOption.NOFOLLOW_LINKS).toMillis() != it) {
                            throw AppError.LocalIo("本地文件修改时间已变化: $relative")
                        }
                    }
                }
            }
        }
    }

    private fun configuredRoot(): Path {
        val config = configStore.load() ?: throw AppError.LocalIo("尚未配置挂载目录")
        if (!config.mountConfigured || config.mountDir.isBlank()) throw AppError.LocalIo("尚未配置挂载目录")
        val root = JvmMountPaths.resolve(config.mountDir)
        if (Files.isSymbolicLink(root) || !Files.isDirectory(root, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("挂载目录不存在或不安全: $root")
        }
        return root.toRealPath()
    }

    private fun safePath(root: Path, relativePath: String): Path {
        validateRelative(relativePath)
        val path = root.resolve(Path.of(relativePath)).normalize()
        if (!path.startsWith(root) || path == root) throw AppError.LocalIo("路径越界: $relativePath")
        var current = root
        for (segment in root.relativize(path)) {
            current = current.resolve(segment)
            if (Files.exists(current, LinkOption.NOFOLLOW_LINKS) && Files.isSymbolicLink(current)) {
                throw AppError.LocalIo("拒绝操作符号链接: $current")
            }
        }
        return path
    }

    private fun requireSafeParent(root: Path, parent: Path) {
        val relative = root.relativize(parent)
        var current = root
        for (segment in relative) {
            current = current.resolve(segment)
            if (!Files.exists(current, LinkOption.NOFOLLOW_LINKS) || Files.isSymbolicLink(current) ||
                !Files.isDirectory(current, LinkOption.NOFOLLOW_LINKS)
            ) throw AppError.LocalIo("目标父目录不存在或不安全: $current")
        }
    }

    private fun rejectUnsafeType(path: Path, item: SyncItem) {
        if (Files.isSymbolicLink(path)) throw AppError.LocalIo("拒绝操作符号链接: $path")
        if (item.isFolder != Files.isDirectory(path, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("本地路径类型与基线不一致: $path")
        }
        if (!item.isFolder && !Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) {
            throw AppError.LocalIo("本地路径不是普通文件: $path")
        }
    }

    private fun moveNoReplace(source: Path, target: Path) {
        try {
            Files.move(source, target, StandardCopyOption.ATOMIC_MOVE)
        } catch (_: AtomicMoveNotSupportedException) {
            Files.move(source, target)
        }
    }

    private fun validateSegment(value: String) {
        require(value.isNotBlank() && value != "." && value != ".." && '/' !in value && '\u0000' !in value) {
            "文件名不合法"
        }
    }

    private fun validateRelative(value: String) {
        val path = Path.of(value)
        require(value.isNotBlank() && !path.isAbsolute && path.none { it.toString() == "." || it.toString() == ".." }) {
            "相对路径不合法: $value"
        }
    }

    private fun inSubtree(path: String, root: String) = path == root || path.startsWith("$root/")
    private fun pathsOverlap(left: String, right: String) = inSubtree(left, right) || inSubtree(right, left)
    private fun parseTime(raw: String): Long? = runCatching { Instant.parse(raw).toEpochMilli() }.getOrNull()
}

internal const val DELETE_TRACE_ERROR_PREFIX = "TRACE_FAILED:"
