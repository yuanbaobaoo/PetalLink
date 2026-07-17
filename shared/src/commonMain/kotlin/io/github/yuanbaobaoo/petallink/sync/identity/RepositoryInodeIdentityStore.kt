package io.github.yuanbaobaoo.petallink.sync.identity

import io.github.yuanbaobaoo.petallink.data.repository.InodeMapRepository
import io.github.yuanbaobaoo.petallink.drive.PlatformTime

/**
 * SQLDelight repository 到同步 identity 端口的唯一适配器。
 */
class RepositoryInodeIdentityStore(
    private val repository: InodeMapRepository,
    private val nowMs: () -> Long = PlatformTime::millis,
) : InodeIdentityStore {
    /**
     * 按 inode 查询身份记录，未找到返回 null
     */
    override suspend fun lookup(inode: ULong): InodeRecord? = repository.lookup(inode)

    /**
     * 写入或更新 inode 到相对路径、fileId 的映射
     */
    override suspend fun upsert(inode: ULong, relativePath: String, fileId: String) {
        repository.upsert(inode, relativePath, fileId, nowMs())
    }

    /**
     * 清理本次扫描未见的 inode 记录
     */
    override suspend fun purgeMissing(seenInodes: Set<ULong>) = repository.purgeMissing(seenInodes)
}
