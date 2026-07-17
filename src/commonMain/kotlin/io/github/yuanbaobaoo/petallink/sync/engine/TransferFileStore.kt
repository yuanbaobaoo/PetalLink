package io.github.yuanbaobaoo.petallink.sync.engine

import kotlinx.serialization.Serializable

/**
 * 下载断点续传元数据：持久化远端文件身份（fileId/size/etag/sha256 等），用于校验断点是否仍有效
 */
@Serializable
data class DownloadResumeMetadata(
    val fileId: String,
    val size: Long,
    val editedTime: String? = null,
    val etag: String? = null,
    val sha256: String? = null,
) {
    /**
     * 是否拥有可稳定识别远端版本的凭据（editedTime / etag / sha256 任一非空）。
     */
    fun hasStableIdentity(): Boolean = !editedTime.isNullOrBlank() || !etag.isNullOrBlank() || !sha256.isNullOrBlank()
}

/**
 * 本地上传源文件快照：记录大小与修改时间，用于检测分片上传期间源文件是否变化
 */
data class LocalSourceSnapshot(val size: Long, val modifiedAtMillis: Long)

/**
 * 传输所需的持久文件操作；JVM 实现负责 fsync 与同目录原子替换。
 */
interface TransferFileStore {
    /**
     * 判断给定路径的文件是否存在。
     */
    suspend fun exists(path: String): Boolean

    /**
     * 返回文件字节大小；路径不存在时由实现决定是否抛异常。
     */
    suspend fun size(path: String): Long

    /**
     * 获取本地源文件快照（大小与最后修改时间），用于上传期间检测源文件是否变化。
     */
    suspend fun snapshot(path: String): LocalSourceSnapshot

    /**
     * 整文件读入内存；文件大小超过 [maxBytes] 时抛异常，防止内存上传越限。
     */
    suspend fun readAll(path: String, maxBytes: Long): ByteArray

    /**
     * 从 [offset] 起读取最多 [maxBytes] 字节，用于分片上传与 Range 续传读取。
     */
    suspend fun readRange(path: String, offset: Long, maxBytes: Int): ByteArray

    /**
     * 计算完整文件的 SHA-256 十六进制摘要。
     */
    suspend fun sha256(path: String, bufferSize: Int = DOWNLOAD_BUFFER_SIZE): String

    /**
     * 读取目的路径的下载续传元数据；不存在或解析失败时返回 null。
     */
    suspend fun readResumeMetadata(destination: String): DownloadResumeMetadata?

    /**
     * 原子写入下载续传元数据（先 fsync 再同目录原子替换）。
     */
    suspend fun writeResumeMetadata(destination: String, metadata: DownloadResumeMetadata)

    /**
     * 删除目的路径的续传元数据及其写入暂存文件。
     */
    suspend fun deleteResumeMetadata(destination: String)

    /**
     * 向下载暂存文件写入分片：[truncate] 为真时先清空，随后从 [offset] 起写入 [bytes]。
     */
    suspend fun writeTemp(destination: String, offset: Long, bytes: ByteArray, truncate: Boolean)

    /**
     * 返回下载暂存文件大小；不存在时返回 null。
     */
    suspend fun tempSize(destination: String): Long?

    /**
     * 删除下载暂存文件（不存在视为成功）。
     */
    suspend fun deleteTemp(destination: String)

    /**
     * 计算下载暂存文件的 SHA-256 十六进制摘要。
     */
    suspend fun sha256Temp(destination: String, bufferSize: Int = DOWNLOAD_BUFFER_SIZE): String

    /**
     * 将下载暂存文件的数据与元数据强制落盘。
     */
    suspend fun fsyncTemp(destination: String)

    /**
     * 将下载暂存文件原子安装到目的路径（替换既有文件并同步父目录）。
     */
    suspend fun installTemp(destination: String)

    companion object {
        /**
         * 默认读写缓冲大小：1 MiB。
         */
        const val DOWNLOAD_BUFFER_SIZE = 1024 * 1024

        /**
         * 由目的路径推导下载暂存文件路径。
         */
        fun tempPath(destination: String) = "$destination.tmp"

        /**
         * 由目的路径推导下载续传元数据文件路径。
         */
        fun metadataPath(destination: String) = "$destination.download-meta.tmp"
    }
}
