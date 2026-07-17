package io.github.yuanbaobaoo.petallink.sync.engine

import kotlinx.serialization.Serializable

@Serializable
data class DownloadResumeMetadata(
    val fileId: String,
    val size: Long,
    val editedTime: String? = null,
    val etag: String? = null,
    val sha256: String? = null,
) {
    fun hasStableIdentity(): Boolean = !editedTime.isNullOrBlank() || !etag.isNullOrBlank() || !sha256.isNullOrBlank()
}

data class LocalSourceSnapshot(val size: Long, val modifiedAtMillis: Long)

/** 传输所需的持久文件操作；JVM 实现负责 fsync 与同目录原子替换。 */
interface TransferFileStore {
    suspend fun exists(path: String): Boolean
    suspend fun size(path: String): Long
    suspend fun snapshot(path: String): LocalSourceSnapshot
    suspend fun readAll(path: String, maxBytes: Long): ByteArray
    suspend fun readRange(path: String, offset: Long, maxBytes: Int): ByteArray
    suspend fun sha256(path: String, bufferSize: Int = DOWNLOAD_BUFFER_SIZE): String

    suspend fun readResumeMetadata(destination: String): DownloadResumeMetadata?
    suspend fun writeResumeMetadata(destination: String, metadata: DownloadResumeMetadata)
    suspend fun deleteResumeMetadata(destination: String)

    suspend fun writeTemp(destination: String, offset: Long, bytes: ByteArray, truncate: Boolean)
    suspend fun tempSize(destination: String): Long?
    suspend fun deleteTemp(destination: String)
    suspend fun sha256Temp(destination: String, bufferSize: Int = DOWNLOAD_BUFFER_SIZE): String
    suspend fun fsyncTemp(destination: String)
    suspend fun installTemp(destination: String)

    companion object {
        const val DOWNLOAD_BUFFER_SIZE = 1024 * 1024
        fun tempPath(destination: String) = "$destination.tmp"
        fun metadataPath(destination: String) = "$destination.download-meta.tmp"
    }
}
