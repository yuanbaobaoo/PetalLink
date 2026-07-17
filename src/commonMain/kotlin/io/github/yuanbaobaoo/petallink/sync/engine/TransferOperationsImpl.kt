package io.github.yuanbaobaoo.petallink.sync.engine

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.data.TransferDirection
import io.github.yuanbaobaoo.petallink.drive.DownloadApi
import io.github.yuanbaobaoo.petallink.drive.ResumeSession
import io.github.yuanbaobaoo.petallink.drive.UploadApi
import io.github.yuanbaobaoo.petallink.drive.UploadProtocol
import io.github.yuanbaobaoo.petallink.drive.validatedResponseOffset
import io.github.yuanbaobaoo.petallink.mount.SkipFilter
import io.ktor.client.statement.bodyAsChannel
import io.ktor.client.statement.readBytes
import io.ktor.http.HttpHeaders
import io.ktor.utils.io.readAvailable
import kotlinx.coroutines.delay
import kotlin.math.min

/**
 * 上传源文件稳定性判定结果：用于预检阶段决定是否允许上传
 */
enum class UploadStability { STABLE, UNSTABLE, EDITING }

/**
 * 上传源稳定性探测接口，用于预检阶段判断文件是否可安全上传
 */
fun interface UploadStabilityProbe {
    /**
     * 探测指定路径文件的上传稳定性
     */
    suspend fun check(path: String): UploadStability
}

/**
 * TransferOperations 的具体实现（对标 task_runner contracts TransferOperations）。
 *
 * 把 TaskRunner 的抽象传输操作连接到实际的 UploadApi/DownloadApi。
 * 详见 docs/06 §TaskRunner。
 *
 * @param uploadApi 上传 API
 * @param downloadApi 下载 API
 * @param readFileBytes 读取本地文件字节（平台注入）
 * @param writeFileBytes 写入本地文件字节（平台注入）
 */
class TransferOperationsImpl(
    private val uploadApi: UploadApi,
    private val downloadApi: DownloadApi,
    private val readFileBytes: suspend (path: String) -> ByteArray,
    private val writeFileBytes: suspend (path: String, ByteArray) -> Unit,
    private val fileExists: suspend (path: String) -> Boolean,
    private val fileSize: suspend (path: String) -> Long,
    private val renameFileImpl: suspend (from: String, to: String) -> Unit = { from, to ->
        platformRenameExpect(from, to)
    },
    private val deleteFileImpl: suspend (path: String) -> Unit = { path ->
        platformDeleteExpect(path)
    },
    private val uploadStability: UploadStabilityProbe? = null,
    private val stabilityPause: suspend (Long) -> Unit = { delay(it) },
    private val fileStore: TransferFileStore? = null,
    private val remoteVerification: (suspend (TaskContext) -> RemoteVerification)? = null,
    private val deleteRemote: suspend (String) -> Unit = { throw AppError.Internal("deleteRemote 未装配") },
) : TransferOperations {

    /**
     * 预检：路径合法性 + 空间 + 冲突。
     */
    override suspend fun preflight(task: TaskContext): PreflightResult {
        // 检查文件名是否应跳过（.hwcloud_ 前缀等）
        val fileName = task.localPath.substringAfterLast("/")
        if (SkipFilter.shouldSkip(fileName)) {
            return PreflightResult.Reject("文件名被跳过规则过滤", io.github.yuanbaobaoo.petallink.sync.TransferState.Failed)
        }
        // 上传预检：本地文件必须存在
        if (task.direction == TransferDirection.UPLOAD) {
            if (!fileExists(task.localPath)) {
                return PreflightResult.Reject("本地文件不存在", io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired)
            }
            uploadStability?.let { probe ->
                for (seconds in listOf(0L, 2L, 3L, 5L)) {
                    if (seconds > 0) stabilityPause(seconds * 1_000)
                    when (probe.check(task.localPath)) {
                        UploadStability.STABLE -> return PreflightResult.Ok
                        UploadStability.EDITING -> return PreflightResult.Reject(
                            "用户正在编辑，等待重新规划",
                            io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired,
                        )
                        UploadStability.UNSTABLE -> Unit
                    }
                }
                return PreflightResult.Reject(
                    "文件尚不稳定，等待重新规划",
                    io.github.yuanbaobaoo.petallink.sync.TransferState.RestartRequired,
                )
            }
        }
        // 下载安装阶段由 TransferFileStore 再校验目标父目录和路径身份。
        return PreflightResult.Ok
    }

    /**
     * 执行传输：上传或下载。
     */
    override suspend fun execute(task: TaskContext, progress: TaskProgressReporter): TaskOutput {
        return when (task.direction) {
            TransferDirection.UPLOAD -> executeUpload(task, progress)
            TransferDirection.DOWNLOAD -> executeDownload(task, progress)
            TransferDirection.DOWNLOAD_UPDATE -> executeDownload(task, progress)
            TransferDirection.DELETE -> executeDelete(task)
        }
    }

    /**
     * 执行远端文件删除；404 视为已删除成功
     */
    private suspend fun executeDelete(task: TaskContext): TaskOutput {
        if (task.fileId.isBlank()) return TaskOutput(TaskDisposition.FAILED, errorMessage = "远端删除缺少 fileId")
        return try {
            deleteRemote(task.fileId)
            TaskOutput(TaskDisposition.COMPLETED, cloudFileId = task.fileId)
        } catch (error: AppError.Remote) {
            if (error.status == 404) TaskOutput(TaskDisposition.COMPLETED, cloudFileId = task.fileId)
            else classifyAndReturn(error)
        } catch (error: AppError) {
            classifyAndReturn(error)
        }
    }

    /**
     * 执行上传
     */
    private suspend fun executeUpload(task: TaskContext, progress: TaskProgressReporter): TaskOutput {
        fileStore?.let { return executeUploadPersistent(task, progress, it) }
        return try {
            val content = readFileBytes(task.localPath)
            // 上报初始进度
            progress.report(0, io.github.yuanbaobaoo.petallink.drive.PlatformTime.millis())

            val fileName = task.localPath.substringAfterLast("/")
            val uploaded = uploadApi.uploadSmall(fileName, task.fileId, content)

            // 上报完成进度
            progress.report(content.size.toLong(), io.github.yuanbaobaoo.petallink.drive.PlatformTime.millis())

            TaskOutput(
                disposition = TaskDisposition.COMPLETED,
                cloudFileId = uploaded.id,
                bytesTransferred = content.size.toLong(),
            )
        } catch (e: AppError) {
            // 上传失败 → 分类
            classifyAndReturn(e)
        }
    }

    /**
     * 持久化大文件上传：分片续传，源文件变化即要求重启
     */
    private suspend fun executeUploadPersistent(
        task: TaskContext,
        progress: TaskProgressReporter,
        store: TransferFileStore,
    ): TaskOutput {
        try {
            val initial = store.snapshot(task.localPath)
            if (task.sourceSize != null && initial.size != task.sourceSize) {
                return TaskOutput(TaskDisposition.RESTART_REQUIRED, errorMessage = "上传源文件大小已变化")
            }
            if (task.sourceMtime != null && initial.modifiedAtMillis != task.sourceMtime) {
                return TaskOutput(TaskDisposition.RESTART_REQUIRED, errorMessage = "上传源文件 mtime 已变化")
            }
            val isUpdate = task.operation == OPERATION_UPDATE
            val fileName = task.localPath.substringAfterLast('/')
            if (initial.size <= UploadProtocol.SMALL_LARGE_THRESHOLD) {
                val bytes = store.readAll(task.localPath, UploadProtocol.SMALL_LARGE_THRESHOLD)
                val uploaded = if (isUpdate) {
                    uploadApi.uploadSmallUpdate(task.fileId, fileName, task.parentFileId, bytes)
                } else {
                    uploadApi.uploadSmall(fileName, task.parentFileId, bytes)
                }
                return TaskOutput(
                    TaskDisposition.COMPLETED,
                    cloudFileId = uploaded.id,
                    bytesTransferred = initial.size,
                )
            }
            if (isUpdate) {
                return TaskOutput(
                    TaskDisposition.RESTART_REQUIRED,
                    errorMessage = ">20MiB Update 不允许降级为 Create",
                )
            }

            var session = if (!task.sessionUrl.isNullOrBlank()) {
                ResumeSession(
                    task.serverId.orEmpty(),
                    task.uploadId.orEmpty(),
                    task.sessionUrl,
                    UploadProtocol.DEFAULT_CHUNK_SIZE,
                )
            } else {
                uploadApi.initResume(fileName, task.parentFileId, initial.size)
            }
            var offset = 0L
            if (!task.sessionUrl.isNullOrBlank()) {
                val status = uploadApi.querySessionStatus(session, initial.size)
                status.sessionUrl?.takeIf(String::isNotBlank)?.let { session = session.copy(sessionUrl = it) }
                status.finalFile?.let {
                    return TaskOutput(TaskDisposition.COMPLETED, it.id, initial.size)
                }
                offset = status.uploaded
            }
            if (!progress.reportResume(session, offset)) {
                return TaskOutput(TaskDisposition.RESTART_REQUIRED, errorMessage = "resume 会话无法持久化")
            }

            var finalPolls = 0
            while (true) {
                val current = store.snapshot(task.localPath)
                if (current != initial) {
                    return TaskOutput(TaskDisposition.RESTART_REQUIRED, errorMessage = "分片上传期间源文件发生变化")
                }
                if (offset < initial.size) {
                    finalPolls = 0
                    val chunkSize = UploadProtocol.validatedChunkSize(session.chunkSize)
                    val length = min(chunkSize, initial.size - offset).toInt()
                    val chunk = store.readRange(task.localPath, offset, length)
                    if (chunk.size != length) {
                        return TaskOutput(TaskDisposition.RESTART_REQUIRED, errorMessage = "读取上传分片时源文件缩短")
                    }
                    val result = uploadApi.putChunk(session, offset, initial.size, chunk)
                    result.sessionUrl?.takeIf(String::isNotBlank)?.let { session = session.copy(sessionUrl = it) }
                    result.finalFile?.let {
                        progress.reportResume(session, initial.size)
                        return TaskOutput(TaskDisposition.COMPLETED, it.id, initial.size)
                    }
                    if (result.uploaded <= offset || result.uploaded > initial.size) {
                        return TaskOutput(
                            TaskDisposition.VERIFYING_REMOTE,
                            errorMessage = "服务端未确认当前分片，禁止本地推算 offset",
                        )
                    }
                    offset = result.uploaded
                    if (!progress.reportResume(session, offset)) {
                        return TaskOutput(TaskDisposition.RESTART_REQUIRED, errorMessage = "确认 offset 无法持久化")
                    }
                    continue
                }

                finalPolls++
                val status = uploadApi.querySessionStatus(session, initial.size)
                status.sessionUrl?.takeIf(String::isNotBlank)?.let { session = session.copy(sessionUrl = it) }
                status.finalFile?.let {
                    progress.reportResume(session, initial.size)
                    return TaskOutput(TaskDisposition.COMPLETED, it.id, initial.size)
                }
                if (status.uploaded < initial.size) {
                    offset = status.uploaded
                    progress.reportResume(session, offset)
                    continue
                }
                if (finalPolls >= UploadProtocol.FINAL_STATUS_MAX_POLLS) {
                    return TaskOutput(
                        TaskDisposition.VERIFYING_REMOTE,
                        errorMessage = "数据范围已确认但最终 File 尚不可核验",
                    )
                }
            }
        } catch (e: AppError.RemoteAmbiguous) {
            return TaskOutput(TaskDisposition.VERIFYING_REMOTE, errorMessage = e.message)
        } catch (e: AppError) {
            return classifyAndReturn(e)
        }
        error("resume 上传循环意外结束")
    }

    /**
     * 执行下载：HTTP 流 → .tmp 临时文件 → sha256 校验 → POSIX rename 原子安装
     */
    private suspend fun executeDownload(task: TaskContext, progress: TaskProgressReporter): TaskOutput {
        fileStore?.let { return executeDownloadPersistent(task, progress, it) }
        return try {
            // 1. 获取远端元数据（含 sha256 校验值、ETag）
            val meta = downloadApi.fetchRemoteMetadata(task.fileId)
            progress.report(0, io.github.yuanbaobaoo.petallink.drive.PlatformTime.millis())

            // 2. 构建内容请求（含 If-Match ETag）
            val response = downloadApi.buildContentRequest(task.fileId, offset = 0, etag = meta.etag)

            // 3. 读取完整响应体到字节（当前为完整下载；Range 续传在后续迭代接入）
            val contentBytes: ByteArray = response.readBytes()

            // 4. 长度校验
            if (contentBytes.size.toLong() != meta.size) {
                throw AppError.Remote(0, "下载长度不匹配: 期望 ${meta.size}, 实际 ${contentBytes.size}")
            }

            // 5. sha256 校验（如有远端提供）
            if (meta.sha256 != null && meta.sha256.isNotBlank()) {
                val actualSha256 = sha256Hex(contentBytes)
                if (!actualSha256.equals(meta.sha256, ignoreCase = true)) {
                    throw AppError.Remote(0, "sha256 校验失败: 期望 ${meta.sha256}, 实际 $actualSha256")
                }
            }

            // 6. 写入 .tmp 临时文件（下载专用后缀，watcher/scanner 忽略）
            val tmpPath = "${task.localPath}.tmp"
            writeFileBytes(tmpPath, contentBytes)

            // 上报完成进度
            progress.report(meta.size, io.github.yuanbaobaoo.petallink.drive.PlatformTime.millis())

            // 7. POSIX rename 原子安装（.tmp → 目标路径，同文件系统保证原子性）
            renameFile(tmpPath, task.localPath)

            TaskOutput(
                disposition = TaskDisposition.COMPLETED,
                bytesTransferred = meta.size,
            )
        } catch (e: AppError) {
            // 下载失败：清理 .tmp 残留
            cleanupTmp(task.localPath)
            classifyAndReturn(e)
        } catch (e: Throwable) {
            cleanupTmp(task.localPath)
            TaskOutput(TaskDisposition.FAILED, errorMessage = "下载失败: ${e.message}")
        }
    }

    /**
     * 持久化大文件下载：Range 断点续传、sha256 校验、版本一致性检查后原子安装
     */
    private suspend fun executeDownloadPersistent(
        task: TaskContext,
        progress: TaskProgressReporter,
        store: TransferFileStore,
    ): TaskOutput {
        var resetAfter416 = false
        try {
            val metadata = downloadApi.fetchRemoteMetadata(task.fileId)
            val identity = metadata.resumeMetadata()
            var offset = store.tempSize(task.localPath) ?: 0L
            val storedIdentity = store.readResumeMetadata(task.localPath)
            if (!identity.hasStableIdentity() || storedIdentity != identity || offset > metadata.size) {
                store.deleteTemp(task.localPath)
                store.deleteResumeMetadata(task.localPath)
                offset = 0L
            }
            store.writeResumeMetadata(task.localPath, identity)

            while (true) {
                val response = downloadApi.buildContentRequest(task.fileId, offset, metadata.etag)
                if (response.status.value == 416 && offset > 0L && !resetAfter416) {
                    store.deleteTemp(task.localPath)
                    store.deleteResumeMetadata(task.localPath)
                    offset = 0L
                    resetAfter416 = true
                    store.writeResumeMetadata(task.localPath, identity)
                    continue
                }
                if (response.status.value == 416) throw AppError.Remote(416, "Range 断点无效")
                val acceptedOffset = validatedResponseOffset(
                    response.status.value,
                    response.headers[HttpHeaders.ContentRange],
                    offset,
                    metadata.size,
                )
                if (acceptedOffset == 0L) store.writeTemp(task.localPath, 0L, ByteArray(0), truncate = true)
                var position = acceptedOffset
                val channel = response.bodyAsChannel()
                val buffer = ByteArray(TransferFileStore.DOWNLOAD_BUFFER_SIZE)
                while (true) {
                    val count = channel.readAvailable(buffer, 0, buffer.size)
                    if (count < 0) break
                    if (count == 0) continue
                    store.writeTemp(task.localPath, position, buffer.copyOf(count), truncate = false)
                    position += count
                    progress.report(position, io.github.yuanbaobaoo.petallink.drive.PlatformTime.millis())
                }
                break
            }

            val actualSize = store.tempSize(task.localPath) ?: 0L
            if (actualSize != metadata.size) {
                throw AppError.Network("下载响应提前结束：期望 ${metadata.size}，实际 $actualSize")
            }
            if (!metadata.sha256.isNullOrBlank()) {
                val actual = store.sha256Temp(task.localPath)
                if (!actual.equals(metadata.sha256, ignoreCase = true)) {
                    throw AppError.Remote(0, "下载 sha256 校验失败")
                }
            }
            val latest = downloadApi.fetchRemoteMetadata(task.fileId)
            if (latest.resumeMetadata() != identity) {
                throw AppError.Conflict("下载期间远端版本已变化")
            }
            store.fsyncTemp(task.localPath)
            store.installTemp(task.localPath)
            store.deleteResumeMetadata(task.localPath)
            return TaskOutput(TaskDisposition.COMPLETED, bytesTransferred = metadata.size)
        } catch (error: AppError) {
            val transient = error is AppError.Network ||
                (error is AppError.Remote && (error.status == 408 || error.status == 429 || error.status in 500..599))
            if (!transient) {
                store.deleteTemp(task.localPath)
                store.deleteResumeMetadata(task.localPath)
            }
            if (error is AppError.Conflict || (error is AppError.Remote && error.status == 412)) {
                return TaskOutput(TaskDisposition.RESTART_REQUIRED, errorMessage = error.message)
            }
            return classifyAndReturn(error)
        } catch (error: Throwable) {
            store.deleteTemp(task.localPath)
            store.deleteResumeMetadata(task.localPath)
            return TaskOutput(TaskDisposition.FAILED, errorMessage = "下载失败: ${error.message}")
        }
    }

    /**
     * sha256 计算为小写十六进制（流式，1MB buffer 对标原项目）
     */
    private fun sha256Hex(data: ByteArray): String {
        // 纯 Kotlin SHA-256 实现（无外部依赖）
        return sha256Pure(data)
    }

    /**
     * POSIX rename（同文件系统原子操作）
     */
    private suspend fun renameFile(from: String, to: String) {
        // 平台注入的 rename 实现（macosMain 用 platform.posix.rename）
        renameFileImpl(from, to)
    }

    /**
     * 清理 .tmp 残留（下载失败时）
     */
    private suspend fun cleanupTmp(localPath: String) {
        val tmpPath = "${localPath}.tmp"
        if (fileExists(tmpPath)) {
            deleteFileImpl(tmpPath)
        }
    }

    /**
     * 远端核验（VerifyingRemote 状态用）
     */
    override suspend fun verifyRemote(task: TaskContext): RemoteVerification {
        remoteVerification?.let { return it(task) }
        return try {
            val candidateId = task.remoteResultFileId?.takeIf(String::isNotBlank)
                ?: task.fileId.takeIf(String::isNotBlank)
                ?: return RemoteVerification.Ambiguous
            val meta = downloadApi.fetchRemoteMetadata(candidateId)
            if (meta.fileId == candidateId && meta.size == task.bytesTotal) {
                RemoteVerification.Committed(meta.fileId)
            } else {
                RemoteVerification.NotCommitted
            }
        } catch (e: AppError.Network) {
            RemoteVerification.Err("网络错误: ${e.message}")
        } catch (e: AppError.Remote) {
            // 404/410 → 会话过期 → 歧义
            if (e.status == 404 || e.status == 410) {
                RemoteVerification.Ambiguous
            } else {
                RemoteVerification.Err("远端错误: ${e.status}")
            }
        } catch (e: Throwable) {
            RemoteVerification.Err("未知错误: ${e.message}")
        }
    }

    /**
     * 错误分类 → 对应 disposition
     */
    private fun classifyAndReturn(e: AppError): TaskOutput {
        if (e is AppError.RemoteAmbiguous) {
            return TaskOutput(TaskDisposition.VERIFYING_REMOTE, errorMessage = e.message)
        }
        return when (e.kind) {
            AppError.ErrorKind.NETWORK -> TaskOutput(
                TaskDisposition.WAITING_FOR_NETWORK, errorMessage = e.message
            )
            AppError.ErrorKind.AUTH -> TaskOutput(
                TaskDisposition.FAILED, errorMessage = "鉴权失败: ${e.message}"
            )
            AppError.ErrorKind.REMOTE -> {
                val status = (e as? AppError.Remote)?.status ?: 0
                when {
                    status == 408 || status == 429 || status in 500..599 -> TaskOutput(
                        TaskDisposition.BACKING_OFF, errorMessage = "服务端错误 $status"
                    )
                    else -> TaskOutput(TaskDisposition.FAILED, errorMessage = "远端错误 $status")
                }
            }
            else -> TaskOutput(TaskDisposition.FAILED, errorMessage = e.message)
        }
    }

    private companion object {
        const val OPERATION_UPDATE = 1
    }
}
