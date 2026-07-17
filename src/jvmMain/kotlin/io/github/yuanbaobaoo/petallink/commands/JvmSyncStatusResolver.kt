package io.github.yuanbaobaoo.petallink.commands

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.config.AppConfig
import io.github.yuanbaobaoo.petallink.config.ConfigStore
import io.github.yuanbaobaoo.petallink.config.JvmMountPaths
import io.github.yuanbaobaoo.petallink.data.SyncItem
import io.github.yuanbaobaoo.petallink.data.repository.SyncItemRepository
import io.github.yuanbaobaoo.petallink.mount.MacXattrAccess
import io.github.yuanbaobaoo.petallink.mount.PlaceholderState
import io.github.yuanbaobaoo.petallink.mount.XattrAccess
import io.github.yuanbaobaoo.petallink.sync.SyncStatus
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path

/** 原 `sync_check_*_local_status` 的 JVM 等价实现。 */
internal class JvmSyncStatusResolver(
    private val configStore: ConfigStore,
    private val repository: SyncItemRepository,
    private val xattrs: XattrAccess = MacXattrAccess,
) {
    /**
     * 解析单个 fileId 对应的本地同步状态（folder / synced / placeholder / not_synced）。
     */
    suspend fun resolveOne(fileId: String): String {
        val item = repository.findByFileId(fileId) ?: return NOT_SYNCED
        if (item.isFolder) return FOLDER
        val root = configuredRootOrNull() ?: return NOT_SYNCED
        return statusWithRoot(item, root)
    }

    /**
     * 批量解析多个 fileId 的本地同步状态，返回 fileId 到状态串的映射。
     */
    suspend fun resolveBatch(fileIds: List<String>): Map<String, String> {
        val root = configuredRootOrNull()
        return fileIds.associateWith { fileId ->
            val item = repository.findByFileId(fileId) ?: return@associateWith NOT_SYNCED
            when {
                item.isFolder -> FOLDER
                root != null -> statusWithRoot(item, root)
                item.status == SyncStatus.SYNCED -> SYNCED
                else -> NOT_SYNCED
            }
        }
    }

    /**
     * 读取配置并返回已挂载根目录的真实路径；未配置或非法时返回 null。
     */
    private fun configuredRootOrNull(): Path? {
        val config = runCatching(configStore::load).getOrNull() ?: return null
        if (!config.mountConfigured || config.mountDir.isBlank()) return null
        val root = runCatching { JvmMountPaths.resolve(config.mountDir) }.getOrNull() ?: return null
        if (Files.isSymbolicLink(root) || !Files.isDirectory(root, LinkOption.NOFOLLOW_LINKS)) return null
        return root.toRealPath()
    }

    /**
     * 在已知根目录下读取本地文件的 xattr 状态，判定是 placeholder 还是 synced。
     */
    private fun statusWithRoot(item: SyncItem, root: Path): String {
        val relative = Path.of(item.localPath)
        if (relative.isAbsolute || relative.none() || relative.any { it.toString() == "." || it.toString() == ".." }) {
            throw AppError.LocalIo("同步项路径非法: ${item.localPath}")
        }
        val path = root.resolve(relative).normalize()
        if (!path.startsWith(root) || path == root) throw AppError.LocalIo("同步项路径越界: ${item.localPath}")
        if (!Files.exists(path, LinkOption.NOFOLLOW_LINKS)) return NOT_SYNCED
        if (Files.isSymbolicLink(path)) throw AppError.LocalIo("拒绝读取符号链接同步状态: $path")
        if (!Files.isRegularFile(path, LinkOption.NOFOLLOW_LINKS)) return NOT_SYNCED
        val raw = xattrs.get(path.toString(), AppConfig.XATTR_STATE)
        val state = raw?.decodeToString()?.trimEnd('\u0000')?.let(PlaceholderState::fromXattr)
        // 命令合同只看 state xattr；即使占位文件被编辑后大小 > 0，UI 仍应标记 placeholder。
        return if (state == PlaceholderState.PLACEHOLDER) PLACEHOLDER else SYNCED
    }

    companion object {
        const val FOLDER = "folder"
        const val SYNCED = "synced"
        const val PLACEHOLDER = "placeholder"
        const val NOT_SYNCED = "not_synced"
    }
}
