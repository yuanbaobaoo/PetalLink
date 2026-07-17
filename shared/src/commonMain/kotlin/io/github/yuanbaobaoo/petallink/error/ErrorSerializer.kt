package io.github.yuanbaobaoo.petallink.error

import io.github.yuanbaobaoo.petallink.AppError

/**
 * 错误扁平序列化（对标 src/error.rs 跨语言合同）。
 *
 * 把 [AppError] 转为 Map，供：
 * - 跨语言合同（如 DELETE_TRACE_ERROR_PREFIX 契约）
 * - IPC 层传递（阶段 6 UI 用）
 * - 日志结构化输出
 */
object ErrorSerializer {

    /**
     * 把 [AppError] 序列化为扁平 Map。
     * 字段：kind、message、status?(Remote)、retryAfterMs?(元数据)
     */
    fun toMap(error: AppError, metadata: ErrorMetadata? = null): Map<String, Any?> {
        val map = mutableMapOf<String, Any?>(
            "kind" to error.kind.name,
            "message" to (error.message ?: ""),
        )
        // Remote 错误携带 HTTP 状态码
        (error as? AppError.Remote)?.let {
            map["status"] = it.status
        }
        // 恢复元数据
        metadata?.retryAfter?.let {
            map["retryAfterMs"] = it.inWholeMilliseconds
        }
        metadata?.transportKind?.let {
            map["transportKind"] = it.name
        }
        return map
    }
}
