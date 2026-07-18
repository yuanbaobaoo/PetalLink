package io.github.yuanbaobaoo.petallink.data

import androidx.room.ConstructedBy
import androidx.room.Database
import androidx.room.RoomDatabase
import androidx.room.RoomDatabaseConstructor
import androidx.room.TypeConverter
import androidx.room.TypeConverters
import androidx.sqlite.driver.bundled.BundledSQLiteDriver
import io.github.yuanbaobaoo.petallink.data.repository.FreeUpStagingRecord
import io.github.yuanbaobaoo.petallink.sync.TransferState
import io.github.yuanbaobaoo.petallink.sync.identity.InodeRecord
import kotlinx.coroutines.Dispatchers

/**
 * Room 跨平台数据库定义。
 *
 * Entity、DAO 与 schema 均位于 `commonMain`；各平台只负责提供数据库文件路径对应的 builder。
 */
@Database(
    entities = [
        SyncItem::class,
        TransferTask::class,
        InodeRecord::class,
        FreeUpStagingRecord::class,
    ],
    version = 1,
    exportSchema = true,
)
@ConstructedBy(PetalLinkDatabaseConstructor::class)
@TypeConverters(DatabaseTypeConverters::class)
abstract class PetalLinkDatabase : RoomDatabase() {
    /**
     * 返回同步基线 DAO。
     */
    abstract fun syncItemDao(): SyncItemDao

    /**
     * 返回传输任务 DAO。
     */
    abstract fun transferTaskDao(): TransferTaskDao

    /**
     * 返回 inode 身份 DAO。
     */
    abstract fun inodeMapDao(): InodeMapDao

    /**
     * 返回释放空间暂存 DAO。
     */
    abstract fun freeUpStagingDao(): FreeUpStagingDao

    /**
     * 返回数据库维护 DAO。
     */
    abstract fun maintenanceDao(): DatabaseMaintenanceDao
}

/**
 * Room 编译器生成的跨平台数据库构造器。
 */
@Suppress("KotlinNoActualForExpect")
expect object PetalLinkDatabaseConstructor : RoomDatabaseConstructor<PetalLinkDatabase> {
    override fun initialize(): PetalLinkDatabase
}

/**
 * Room 使用的 common 类型转换器。
 */
class DatabaseTypeConverters {
    /**
     * 将传输方向编码为稳定序号。
     */
    @TypeConverter
    fun transferDirectionToInt(value: TransferDirection): Int = value.ordinal

    /**
     * 从稳定序号恢复传输方向。
     */
    @TypeConverter
    fun intToTransferDirection(value: Int): TransferDirection =
        TransferDirection.entries.getOrElse(value) { TransferDirection.UPLOAD }

    /**
     * 将传输状态编码为稳定序号。
     */
    @TypeConverter
    fun transferStateToInt(value: TransferState): Int = value.ordinal

    /**
     * 从稳定序号恢复传输状态。
     */
    @TypeConverter
    fun intToTransferState(value: Int): TransferState =
        TransferState.entries.getOrElse(value) { TransferState.Pending }

    /**
     * 将无符号 inode 映射为 SQLite INTEGER。
     */
    @TypeConverter
    fun ulongToLong(value: ULong): Long = value.toLong()

    /**
     * 从 SQLite INTEGER 恢复无符号 inode。
     */
    @TypeConverter
    fun longToULong(value: Long): ULong = value.toULong()
}

/**
 * 使用统一 bundled SQLite 驱动构建 Room 数据库。
 *
 * @param builder 当前平台创建的数据库 builder。
 * @return 已初始化的 Room 数据库。
 */
internal fun buildPetalLinkDatabase(
    builder: RoomDatabase.Builder<PetalLinkDatabase>,
): PetalLinkDatabase = builder
    .setDriver(BundledSQLiteDriver())
    .setQueryCoroutineContext(Dispatchers.IO)
    .build()

/**
 * 按当前平台文件系统规则创建 Room 数据库 builder。
 *
 * @param dbPath SQLite 文件绝对路径。
 * @return 尚未配置驱动的 Room builder。
 */
internal expect fun createPetalLinkDatabaseBuilder(
    dbPath: String,
): RoomDatabase.Builder<PetalLinkDatabase>

/**
 * 返回当前平台的 Unix 毫秒时间戳。
 */
internal expect fun databaseCurrentTimeMillis(): Long
