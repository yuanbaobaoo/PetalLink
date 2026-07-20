package io.github.yuanbaobaoo.petallink.data

import io.github.yuanbaobaoo.petallink.data.repository.FreeUpStagingRepository
import io.github.yuanbaobaoo.petallink.data.repository.InodeMapRepository
import io.github.yuanbaobaoo.petallink.data.repository.SyncItemRepository
import io.github.yuanbaobaoo.petallink.data.repository.TransferRepository
import androidx.room.useWriterConnection

/**
 * 应用数据库访问入口。
 *
 * 数据模型、DAO、Repository 和事务语义均位于 `commonMain`。平台层只提供 Room builder，
 * 因此增加 Kotlin/Native target 时不会产生第二套业务类型或仓库实现。
 *
 * @param dbPath SQLite 数据库文件绝对路径。
 */
class PetalLinkDb(dbPath: String) {
    private val database = buildPetalLinkDatabase(createPetalLinkDatabaseBuilder(dbPath))

    /** 同步基线仓库。 */
    val syncItems: SyncItemRepository = RoomSyncItemRepository(database.syncItemDao())

    /** 持久化传输任务仓库。 */
    val transfers: TransferRepository = RoomTransferRepository(database.transferTaskDao())

    /** inode 身份映射仓库。 */
    val inodeMap: InodeMapRepository = RoomInodeMapRepository(database.inodeMapDao())

    /** 释放空间暂存仓库。 */
    val freeUpStaging: FreeUpStagingRepository = RoomFreeUpStagingRepository(database.freeUpStagingDao())

    /**
     * 原子清理全部账号运行数据。
     */
    suspend fun clearAll() = clearMountState()

    /**
     * 原子清理挂载目录关联的同步、传输和身份状态。
     */
    suspend fun clearMountState() {
        database.maintenanceDao().clearMountState()
    }

    /**
     * 在单个写事务内执行块（用于基线批量结算原子化，对标 settlement.rs:297-375 的同事务提交）。
     */
    suspend fun <T> withTransaction(block: suspend () -> T): T =
        database.useWriterConnection { conn ->
            conn.withTransaction(androidx.room.Transactor.SQLiteTransactionType.IMMEDIATE) { block() }
        }

    /**
     * 关闭数据库并释放底层 SQLite 资源。
     */
    fun close() {
        database.close()
    }
}
