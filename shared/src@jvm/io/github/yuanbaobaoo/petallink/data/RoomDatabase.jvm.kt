package io.github.yuanbaobaoo.petallink.data

import androidx.room.Room
import androidx.room.RoomDatabase
import java.nio.file.Files
import java.nio.file.Path

internal actual fun createPetalLinkDatabaseBuilder(
    dbPath: String,
): RoomDatabase.Builder<PetalLinkDatabase> {
    val path = Path.of(dbPath).toAbsolutePath().normalize()
    path.parent?.let(Files::createDirectories)
    if (Files.notExists(path)) Files.createFile(path)
    return Room.databaseBuilder(name = path.toString())
}

/**
 * JVM 当前 Unix 毫秒时间戳。
 */
internal actual fun databaseCurrentTimeMillis(): Long = System.currentTimeMillis()
