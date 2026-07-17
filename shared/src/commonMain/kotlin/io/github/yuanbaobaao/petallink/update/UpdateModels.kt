package io.github.yuanbaobaao.petallink.update

import kotlinx.serialization.Serializable

@Serializable
data class UpdateManifest(
    val version: String,
    val url: String,
    val sha256: String,
    val notes: String = "",
    val minimumSystemVersion: String = "12.0",
)

data class SemanticVersion(val major: Int, val minor: Int, val patch: Int) : Comparable<SemanticVersion> {
    override fun compareTo(other: SemanticVersion): Int =
        compareValuesBy(this, other, SemanticVersion::major, SemanticVersion::minor, SemanticVersion::patch)

    companion object {
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

fun UpdateManifest.isNewerThan(currentVersion: String): Boolean {
    val current = SemanticVersion.parse(currentVersion) ?: return false
    val offered = SemanticVersion.parse(version) ?: return false
    return offered > current
}

class TransferIdleWaiter(
    private val hasActiveTransfers: () -> Boolean,
    private val nowMs: () -> Long,
    private val pause: suspend (Long) -> Unit,
) {
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
