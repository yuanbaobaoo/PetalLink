package io.github.yuanbaobaoo.petallink.sync.identity

/**
 * 基于 inode 的文件身份识别（对标原项目 inode 方案，docs/11 §4.1）
 *
 * 取代旧的 fileId xattr 机制。inode 是文件系统内部编号：
 * - mv 改名时 inode 不变 → 天然识别"移动"
 * - cp 复制时 inode 产生新编号 → 副本自动当新文件处理
 *
 * 由此"同一身份出现在多处"在结构上不可能发生，整套 fileId 兜底逻辑可删除。
 * 详见 docs/11-基于inode的文件身份识别方案.md。
 */

/**
 * inode 映射记录：一个本地 inode ↔ 云端身份的对应关系。
 * 对应数据库表 local_inode_map（docs/11 §3.1）。
 */
data class InodeRecord(
    val inode: ULong,           // 文件系统 inode（macOS meta.ino() 为 u64）
    val relativePath: String,   // 相对挂载目录的路径
    val fileId: String,         // 云端文件 ID
    val scannedAt: Long,        // 上次扫描到该 inode 的时间戳（ms）
)

/**
 * inode 身份存储接口（平台/数据层实现）。
 * 所有身份查询都走此接口——只读 DB 操作，不碰文件 xattr，不涉及"补写自愈"。
 */
interface InodeIdentityStore {

    /**
     * 查询某 inode 对应的云端身份。
     * 用于扫描时识别重命名：同 inode 出现在新路径 = 移动。
     */
    suspend fun lookup(inode: ULong): InodeRecord?

    /**
     * 下载/释放空间完成后主动更新映射（程序自己操作文件时的确定性记账）。
     * 替代旧方案的 set_file_id_xattr 补写——要么成功要么回滚，不再静默丢失。
     */
    suspend fun upsert(inode: ULong, relativePath: String, fileId: String)

    /**
     * 扫描结束后，根据本轮见到的 inode 集合清理陈旧记录。
     */
    suspend fun purgeMissing(seenInodes: Set<ULong>)
}

/**
 * 扫描快照中基于稳定 inode 配对出的本地移动。
 */
data class DetectedMove(
    val inode: ULong,
    val fileId: String,
    val oldRelativePath: String,
    val newRelativePath: String,
)

/**
 * 基于稳定 inode 的文件移动检测器，比对扫描快照识别本地文件的移动
 */
object InodeMoveDetector {
    /**
     * 同 inode 且路径改变时才输出 move；copy 产生新 inode，因而不会被误判为 move。
     * 成功识别后立即更新路径映射，但不自动 purge，由完整扫描提交者统一执行。
     */
    suspend fun detectMoves(
        entries: Collection<io.github.yuanbaobaoo.petallink.mount.LocalFileEntry>,
        identity: InodeIdentityStore,
    ): List<DetectedMove> {
        val result = mutableListOf<DetectedMove>()
        for (entry in entries.sortedBy { it.relativePath }) {
            val old = identity.lookup(entry.inode) ?: continue
            if (old.relativePath == entry.relativePath) continue
            result += DetectedMove(entry.inode, old.fileId, old.relativePath, entry.relativePath)
            identity.upsert(entry.inode, entry.relativePath, old.fileId)
        }
        return result
    }
}
