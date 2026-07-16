package io.github.yuanbaobaao.petallink.data

import app.cash.sqldelight.db.SqlDriver
import app.cash.sqldelight.driver.jdbc.sqlite.JdbcSqliteDriver
import io.github.yuanbaobaao.petallink.data.repository.FreeUpStagingRepository
import io.github.yuanbaobaao.petallink.data.repository.FreeUpStagingRepositoryImpl
import io.github.yuanbaobaao.petallink.data.repository.InodeMapRepository
import io.github.yuanbaobaao.petallink.data.repository.InodeMapRepositoryImpl
import io.github.yuanbaobaao.petallink.data.repository.SyncItemRepository
import io.github.yuanbaobaao.petallink.data.repository.SyncItemRepositoryImpl
import io.github.yuanbaobaao.petallink.data.repository.TransferRepository
import io.github.yuanbaobaao.petallink.data.repository.TransferRepositoryImpl

/**
 * JVM 数据库实现（actual）。
 * 用 SQLDelight JdbcSqliteDriver（sqlite-jdbc）。
 */
actual class PetalLinkDb actual constructor(dbPath: String) {
    private val driver: SqlDriver = JdbcSqliteDriver("jdbc:sqlite:$dbPath")
    private val database: PetalLinkDatabase = PetalLinkDatabase(driver)

    actual val syncItems: SyncItemRepository = SyncItemRepositoryImpl(database.sync_itemsQueries)
    actual val transfers: TransferRepository = TransferRepositoryImpl(database.transfer_queueQueries)
    actual val inodeMap: InodeMapRepository = InodeMapRepositoryImpl(database.local_inode_mapQueries)
    actual val freeUpStaging: FreeUpStagingRepository = FreeUpStagingRepositoryImpl(database.free_up_stagingQueries)

    actual fun close() {
        driver.close()
    }
}
