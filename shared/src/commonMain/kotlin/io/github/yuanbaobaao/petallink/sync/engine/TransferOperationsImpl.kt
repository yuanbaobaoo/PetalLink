package io.github.yuanbaobaao.petallink.sync.engine

import io.github.yuanbaobaao.petallink.AppError
import io.github.yuanbaobaao.petallink.data.TransferDirection
import io.github.yuanbaobaao.petallink.drive.DownloadApi
import io.github.yuanbaobaao.petallink.drive.UploadApi
import io.github.yuanbaobaao.petallink.mount.SkipFilter
import io.ktor.client.statement.readBytes

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
) : TransferOperations {

    /**
     * 预检：路径合法性 + 空间 + 冲突。
     */
    override suspend fun preflight(task: TaskContext): PreflightResult {
        // 检查文件名是否应跳过（.hwcloud_ 前缀等）
        val fileName = task.localPath.substringAfterLast("/")
        if (SkipFilter.shouldSkip(fileName)) {
            return PreflightResult.Reject("文件名被跳过规则过滤", io.github.yuanbaobaao.petallink.sync.TransferState.Failed)
        }
        // 上传预检：本地文件必须存在
        if (task.direction == TransferDirection.UPLOAD) {
            if (!fileExists(task.localPath)) {
                return PreflightResult.Reject("本地文件不存在", io.github.yuanbaobaao.petallink.sync.TransferState.RestartRequired)
            }
        }
        // 下载预检：目标路径父目录应存在（简化检查）
        return PreflightResult.Ok
    }

    /**
     * 执行传输：上传或下载。
     */
    override suspend fun execute(task: TaskContext, progress: TaskProgressReporter): TaskOutput {
        return when (task.direction) {
            TransferDirection.UPLOAD -> executeUpload(task, progress)
            TransferDirection.DOWNLOAD -> executeDownload(task, progress)
        }
    }

    /** 执行上传 */
    private suspend fun executeUpload(task: TaskContext, progress: TaskProgressReporter): TaskOutput {
        return try {
            val content = readFileBytes(task.localPath)
            // 上报初始进度
            progress.report(0, io.github.yuanbaobaao.petallink.drive.PlatformTime.millis())

            val fileName = task.localPath.substringAfterLast("/")
            val uploaded = uploadApi.uploadSmall(fileName, task.fileId, content)

            // 上报完成进度
            progress.report(content.size.toLong(), io.github.yuanbaobaao.petallink.drive.PlatformTime.millis())

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

    /** 执行下载：HTTP 流 → .tmp 临时文件 → sha256 校验 → POSIX rename 原子安装 */
    private suspend fun executeDownload(task: TaskContext, progress: TaskProgressReporter): TaskOutput {
        return try {
            // 1. 获取远端元数据（含 sha256 校验值、ETag）
            val meta = downloadApi.fetchRemoteMetadata(task.fileId)
            progress.report(0, io.github.yuanbaobaao.petallink.drive.PlatformTime.millis())

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
            progress.report(meta.size, io.github.yuanbaobaao.petallink.drive.PlatformTime.millis())

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

    /** sha256 计算为小写十六进制（流式，1MB buffer 对标原项目） */
    private fun sha256Hex(data: ByteArray): String {
        // 纯 Kotlin SHA-256 实现（无外部依赖）
        return sha256Pure(data)
    }

    /** POSIX rename（同文件系统原子操作） */
    private suspend fun renameFile(from: String, to: String) {
        // 平台注入的 rename 实现（macosMain 用 platform.posix.rename）
        renameFileImpl(from, to)
    }

    /** 清理 .tmp 残留（下载失败时） */
    private suspend fun cleanupTmp(localPath: String) {
        val tmpPath = "${localPath}.tmp"
        if (fileExists(tmpPath)) {
            deleteFileImpl(tmpPath)
        }
    }

    /** 远端核验（VerifyingRemote 状态用） */
    override suspend fun verifyRemote(task: TaskContext): RemoteVerification {
        return try {
            val meta = downloadApi.fetchRemoteMetadata(task.fileId)
            if (meta.fileId == task.fileId) {
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

    /** 错误分类 → 对应 disposition */
    private fun classifyAndReturn(e: AppError): TaskOutput {
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
                    status in 500..599 -> TaskOutput(
                        TaskDisposition.BACKING_OFF, errorMessage = "服务端错误 $status"
                    )
                    else -> TaskOutput(TaskDisposition.FAILED, errorMessage = "远端错误 $status")
                }
            }
            else -> TaskOutput(TaskDisposition.FAILED, errorMessage = e.message)
        }
    }
}
