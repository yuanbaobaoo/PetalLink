package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.drive.DriveFile
import io.github.yuanbaobaoo.petallink.drive.DriveParsers
import io.github.yuanbaobaoo.petallink.drive.FilesApi
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import java.time.Instant

/**
 * 对不确定远端写执行只读收敛；Create 永不因“查不到”而直接重放。
 */
class JvmRemoteTransferVerifier(
    private val filesApi: FilesApi,
    private val fileStore: TransferFileStore,
) {
    /**
     * 对不确定的远端写执行只读收敛核验，判定任务应提交、重放还是保持歧义。
     */
    suspend fun verify(task: TaskContext): RemoteVerification {
        return try {
        if (task.direction == TransferDirection.DELETE) {
            if (task.fileId.isBlank()) return RemoteVerification.Err("Delete 缺少 fileId")
            return if (filesApi.verifyDeleted(task.fileId)) RemoteVerification.Committed(task.fileId)
            else RemoteVerification.NotCommitted
        }
        val persisted = task.remoteResultFileId?.takeIf(String::isNotBlank)
        if (persisted != null) {
            val file = filesApi.getFile(persisted)
            return if (matchesIdentity(file, task)) RemoteVerification.Committed(persisted)
            else RemoteVerification.Ambiguous
        }

        val isUpdate = task.operation == 1
        if (isUpdate) {
            if (task.fileId.isBlank()) return RemoteVerification.Err("Update 缺少 fileId")
            val file = filesApi.getFile(task.fileId)
            if (!matchesIdentity(file, task)) return RemoteVerification.Ambiguous
            val edited = file.editedTime?.let { Instant.parse(it).toEpochMilli() }
                ?: return RemoteVerification.Ambiguous
            return if (task.expectedCloudEditedTime != null && edited == task.expectedCloudEditedTime) {
                RemoteVerification.NotCommitted
            } else {
                RemoteVerification.Committed(task.fileId)
            }
        }

        // Create 没有 persisted id：按原请求意图只读查重。0 个和多个都保持 Ambiguous。
        val localHash = runCatching { fileStore.sha256(task.localPath) }.getOrNull()
        val lower = task.createdAt - 120_000L
        val upper = task.createdAt + 30L * 24 * 60 * 60 * 1_000
        val matches = filesApi.listAllFiles(task.parentFileId).filter { file ->
            val created = file.createdTime?.let { runCatching { Instant.parse(it).toEpochMilli() }.getOrNull() }
            matchesIdentity(file, task) &&
                created != null && created in lower..upper &&
                (file.contentHash.isNullOrBlank() || localHash != null && file.contentHash.equals(localHash, true))
        }
        if (matches.size == 1) RemoteVerification.Committed(matches.single().id!!)
        else RemoteVerification.Ambiguous
    } catch (error: AppError.Network) {
        RemoteVerification.Err(error.message ?: "network")
    } catch (error: AppError.Remote) {
        if (error.status == 404 && task.operation == 1) RemoteVerification.NotCommitted
        else if (error.status == 404) RemoteVerification.Ambiguous
        else RemoteVerification.Err(error.message ?: "remote ${error.status}")
        } catch (error: Throwable) {
            RemoteVerification.Err(error.message ?: "verify failed")
        }
    }

    /**
     * 比对远端文件与任务的标识（id、名称、大小、父目录）是否一致。
     */
    private fun matchesIdentity(file: DriveFile, task: TaskContext): Boolean {
        if (file.id.isNullOrBlank() || file.name != task.localPath.substringAfterLast('/') || file.sizeBytes != task.bytesTotal) {
            return false
        }
        if (!task.parentFileId.isNullOrBlank()) {
            val parent = runCatching { DriveParsers.singleParent(file, "remote verify") }.getOrNull()
            if (parent != task.parentFileId) return false
        }
        return true
    }
}
