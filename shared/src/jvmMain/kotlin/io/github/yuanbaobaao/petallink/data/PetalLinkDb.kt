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
import java.nio.file.Files
import java.nio.file.Path
import java.sql.Connection
import java.sql.DriverManager

/**
 * JVM 数据库实现（actual）。
 * 用 SQLDelight JdbcSqliteDriver（sqlite-jdbc）。
 */
actual class PetalLinkDb actual constructor(dbPath: String) {
    private val driver: SqlDriver = DatabaseBootstrap.open(dbPath)
    private val database: PetalLinkDatabase = PetalLinkDatabase(driver)

    actual val syncItems: SyncItemRepository = SyncItemRepositoryImpl(database.sync_itemsQueries)
    actual val transfers: TransferRepository = TransferRepositoryImpl(database.transfer_queueQueries)
    actual val inodeMap: InodeMapRepository = InodeMapRepositoryImpl(database.local_inode_mapQueries)
    actual val freeUpStaging: FreeUpStagingRepository = FreeUpStagingRepositoryImpl(database.free_up_stagingQueries)

    actual fun clearAll() = clearMountState()

    actual fun clearMountState() {
        database.transaction {
            listOf(
                "DELETE FROM transfer_queue",
                "DELETE FROM sync_items",
                "DELETE FROM sync_cursor",
                "DELETE FROM local_inode_map",
                "DELETE FROM free_up_staging",
            ).forEach { driver.execute(null, it, 0) }
        }
    }

    actual fun close() {
        driver.close()
    }
}

/** SQLite 首次建库与 v1→v6 原子迁移。 */
internal object DatabaseBootstrap {
    const val SCHEMA_VERSION = 6

    fun open(dbPath: String): SqlDriver {
        val path = Path.of(dbPath).toAbsolutePath().normalize()
        path.parent?.let(Files::createDirectories)
        val url = "jdbc:sqlite:$path"
        val status = DriverManager.getConnection(url).use { connection ->
            val version = connection.createStatement().use { statement ->
                statement.executeQuery("PRAGMA user_version").use { rows -> rows.getInt(1) }
            }
            val hasCoreTables = tableExists(connection, "sync_items") || tableExists(connection, "transfer_queue")
            version to hasCoreTables
        }
        require(status.first <= SCHEMA_VERSION) {
            "数据库版本 ${status.first} 高于当前支持版本 $SCHEMA_VERSION"
        }

        if (status.second) migrate(url, if (status.first == 0) 1 else status.first)

        val driver = JdbcSqliteDriver(url)
        if (!status.second) {
            PetalLinkDatabase.Schema.create(driver)
            driver.execute(null, "PRAGMA user_version = $SCHEMA_VERSION", 0)
        }
        driver.execute(null, "PRAGMA foreign_keys = ON", 0)
        return driver
    }

    private fun migrate(url: String, fromVersion: Int) {
        if (fromVersion == SCHEMA_VERSION) return
        DriverManager.getConnection(url).use { connection ->
            connection.autoCommit = false
            try {
                var version = fromVersion
                if (version < 2) {
                    addColumn(connection, "transfer_queue", "server_id", "TEXT")
                    addColumn(connection, "transfer_queue", "upload_id", "TEXT")
                    addColumn(connection, "transfer_queue", "resume_offset", "INTEGER NOT NULL DEFAULT 0")
                    version = 2
                }
                if (version < 3) {
                    addColumn(connection, "sync_items", "local_size", "INTEGER")
                    version = 3
                }
                if (version < 4) {
                    addColumn(connection, "transfer_queue", "session_url", "TEXT")
                    version = 4
                }
                if (version < 5) {
                    addColumn(connection, "transfer_queue", "relative_path", "TEXT")
                    addColumn(connection, "transfer_queue", "parent_file_id", "TEXT")
                    addColumn(connection, "transfer_queue", "operation", "INTEGER")
                    addColumn(connection, "transfer_queue", "source_mtime", "INTEGER")
                    addColumn(connection, "transfer_queue", "source_size", "INTEGER")
                    addColumn(connection, "transfer_queue", "expected_cloud_edited_time", "INTEGER")
                    addColumn(connection, "transfer_queue", "attempt_count", "INTEGER NOT NULL DEFAULT 0")
                    addColumn(connection, "transfer_queue", "next_retry_at", "INTEGER")
                    addColumn(connection, "transfer_queue", "error_kind", "INTEGER")
                    addColumn(connection, "transfer_queue", "remote_result_file_id", "TEXT")
                    addColumn(connection, "transfer_queue", "state_revision", "INTEGER NOT NULL DEFAULT 0")
                    connection.createStatement().use { statement ->
                        statement.executeUpdate(
                            "UPDATE transfer_queue SET relative_path = local_path " +
                                "WHERE relative_path IS NULL AND local_path IS NOT NULL",
                        )
                        statement.executeUpdate(
                            "UPDATE transfer_queue SET error_kind = 11 WHERE state = 4 AND error_kind IS NULL",
                        )
                        statement.executeUpdate(
                            "UPDATE transfer_queue SET state = CASE state " +
                                "WHEN 0 THEN 0 WHEN 1 THEN 0 WHEN 2 THEN 0 WHEN 3 THEN 6 " +
                                "WHEN 4 THEN 7 WHEN 5 THEN 8 ELSE 7 END",
                        )
                    }
                    version = 5
                }
                if (version < 6) {
                    createV6Tables(connection)
                    version = 6
                }
                createTerminalIndexes(connection)
                connection.createStatement().use { it.execute("PRAGMA user_version = $version") }
                connection.commit()
            } catch (error: Throwable) {
                connection.rollback()
                throw IllegalStateException("数据库迁移失败：v$fromVersion → v$SCHEMA_VERSION", error)
            }
        }
    }

    private fun addColumn(connection: Connection, table: String, column: String, definition: String) {
        if (columnExists(connection, table, column)) return
        connection.createStatement().use { it.execute("ALTER TABLE $table ADD COLUMN $column $definition") }
    }

    private fun tableExists(connection: Connection, table: String): Boolean =
        connection.prepareStatement("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?").use { query ->
            query.setString(1, table)
            query.executeQuery().use { it.next() }
        }

    private fun columnExists(connection: Connection, table: String, column: String): Boolean =
        connection.createStatement().use { statement ->
            statement.executeQuery("PRAGMA table_info($table)").use { rows ->
                while (rows.next()) if (rows.getString("name") == column) return@use true
                false
            }
        }

    private fun createV6Tables(connection: Connection) {
        connection.createStatement().use { statement ->
            statement.execute(
                """CREATE TABLE IF NOT EXISTS local_inode_map (
                    inode INTEGER NOT NULL PRIMARY KEY,
                    relative_path TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    scanned_at INTEGER NOT NULL
                )""".trimIndent(),
            )
            statement.execute(
                """CREATE TABLE IF NOT EXISTS free_up_staging (
                    staging_name TEXT NOT NULL PRIMARY KEY,
                    relative_path TEXT NOT NULL,
                    file_id TEXT NOT NULL,
                    source_mtime INTEGER,
                    source_size INTEGER,
                    created_at INTEGER NOT NULL
                )""".trimIndent(),
            )
        }
    }

    private fun createTerminalIndexes(connection: Connection) {
        val statements = listOf(
            "CREATE INDEX IF NOT EXISTS idx_sync_items_local_path ON sync_items(local_path)",
            "CREATE INDEX IF NOT EXISTS idx_sync_items_parent ON sync_items(parent_folder_id)",
            "CREATE INDEX IF NOT EXISTS idx_transfer_queue_state ON transfer_queue(state)",
            "CREATE INDEX IF NOT EXISTS idx_transfer_queue_direction ON transfer_queue(direction)",
            "CREATE INDEX IF NOT EXISTS idx_transfer_queue_file_id ON transfer_queue(file_id)",
            "CREATE INDEX IF NOT EXISTS idx_transfer_queue_relative ON transfer_queue(relative_path)",
            "CREATE INDEX IF NOT EXISTS idx_transfer_queue_next_retry ON transfer_queue(next_retry_at)",
            "CREATE INDEX IF NOT EXISTS idx_inode_map_path ON local_inode_map(relative_path)",
            "CREATE INDEX IF NOT EXISTS idx_inode_map_fid ON local_inode_map(file_id)",
        )
        connection.createStatement().use { statement -> statements.forEach(statement::execute) }
    }
}
