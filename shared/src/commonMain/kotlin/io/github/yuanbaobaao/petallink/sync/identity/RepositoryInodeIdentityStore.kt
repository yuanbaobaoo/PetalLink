package io.github.yuanbaobaao.petallink.sync.identity

import io.github.yuanbaobaao.petallink.data.repository.InodeMapRepository
import io.github.yuanbaobaao.petallink.drive.PlatformTime

/** SQLDelight repository 到同步 identity 端口的唯一适配器。 */
class RepositoryInodeIdentityStore(
    private val repository: InodeMapRepository,
    private val nowMs: () -> Long = PlatformTime::millis,
) : InodeIdentityStore {
    override suspend fun lookup(inode: ULong): InodeRecord? = repository.lookup(inode)

    override suspend fun upsert(inode: ULong, relativePath: String, fileId: String) {
        repository.upsert(inode, relativePath, fileId, nowMs())
    }

    override suspend fun purgeMissing(seenInodes: Set<ULong>) = repository.purgeMissing(seenInodes)
}
