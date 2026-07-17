package io.github.yuanbaobaao.petallink.data

import io.github.yuanbaobaao.petallink.data.repository.FreeUpStagingRepository
import io.github.yuanbaobaao.petallink.data.repository.InodeMapRepository
import io.github.yuanbaobaao.petallink.data.repository.SyncItemRepository
import io.github.yuanbaobaao.petallink.data.repository.TransferRepository

/**
 * 数据库访问入口（expect，macosMain 提供 actual）。
 *
 * actual 负责创建 SQLDelight NativeSqliteDriver + PetalLinkDatabase，
 * 并把四张表的查询封装为 repository。所有 repository 共享同一个 driver/事务。
 *
 * @param dbPath SQLite 数据库文件绝对路径（macosMain 传入 Application Support 目录下的路径）
 */
expect class PetalLinkDb(dbPath: String) {
    val syncItems: SyncItemRepository
    val transfers: TransferRepository
    val inodeMap: InodeMapRepository
    val freeUpStaging: FreeUpStagingRepository

    /** 登出/清空应用时原子清理全部账号相关运行数据。 */
    fun clearAll()

    /** 切换同步根目录时原子清理所有挂载目录绑定状态。 */
    fun clearMountState()

    /** 关闭数据库连接（应用退出时调用） */
    fun close()
}
