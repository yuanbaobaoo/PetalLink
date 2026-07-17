package io.github.yuanbaobaoo.petallink.update

import kotlinx.serialization.Serializable

/**
 * 从远端拉取的应用更新清单，包含版本号、安装包地址、校验值及最低系统版本等
 */
@Serializable
data class UpdateManifest(
    val version: String,
    val url: String,
    val sha256: String,
    val notes: String = "",
    val minimumSystemVersion: String = "12.0",
)

/**
 * 语义化版本号（major.minor.patch），支持解析与比较
 */
data class SemanticVersion(val major: Int, val minor: Int, val patch: Int) : Comparable<SemanticVersion> {
    /**
     * 按 major、minor、patch 依次比较两个语义化版本
     */
    override fun compareTo(other: SemanticVersion): Int =
        compareValuesBy(this, other, SemanticVersion::major, SemanticVersion::minor, SemanticVersion::patch)

    companion object {
        /**
         * 解析形如 v1.2.3 的版本字符串，失败返回 null
         */
        fun parse(value: String): SemanticVersion? {
            val match = Regex("^v?(\\d+)\\.(\\d+)\\.(\\d+)(?:[-+].*)?$").matchEntire(value.trim()) ?: return null
            return SemanticVersion(
                match.groupValues[1].toIntOrNull() ?: return null,
                match.groupValues[2].toIntOrNull() ?: return null,
                match.groupValues[3].toIntOrNull() ?: return null,
            )
        }
    }
}

/**
 * 判断本清单版本是否高于当前版本
 */
fun UpdateManifest.isNewerThan(currentVersion: String): Boolean {
    val current = SemanticVersion.parse(currentVersion) ?: return false
    val offered = SemanticVersion.parse(version) ?: return false
    return offered > current
}

/**
 * 等待传输任务进入空闲的工具，通过轮询检查活跃任务并在超时前阻塞挂起
 */
class TransferIdleWaiter(
    private val hasActiveTransfers: () -> Boolean,
    private val nowMs: () -> Long,
    private val pause: suspend (Long) -> Unit,
) {
    /**
     * 轮询等待传输任务进入空闲；超时则返回 false
     */
    suspend fun await(timeoutMs: Long = MAX_WAIT_MS, pollMs: Long = POLL_MS): Boolean {
        val deadline = nowMs() + timeoutMs
        while (hasActiveTransfers()) {
            if (nowMs() >= deadline) return false
            pause(pollMs.coerceAtMost((deadline - nowMs()).coerceAtLeast(1L)))
        }
        return true
    }

    companion object {
        const val MAX_WAIT_MS = 300_000L
        const val POLL_MS = 2_000L
    }
}
