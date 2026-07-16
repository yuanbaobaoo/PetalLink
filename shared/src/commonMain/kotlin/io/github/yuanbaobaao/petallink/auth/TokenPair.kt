package io.github.yuanbaobaao.petallink.auth

/**
 * Token 对（对标原项目 auth/models.rs TokenPair）
 *
 * 详见 docs/07 §token.bin。
 */
data class TokenPair(
    val accessToken: String,
    val refreshToken: String,
    val expiresAt: Long,        // 毫秒时间戳
    val tokenType: String = "Bearer",
    val scope: String? = null,
) {
    /** 是否已过期 */
    fun isExpired(nowMs: Long): Boolean = nowMs >= expiresAt

    /** 是否将在 [bufferSecs] 秒内过期（提前刷新用） */
    fun willExpireWithin(bufferSecs: Long, nowMs: Long): Boolean =
        nowMs + bufferSecs * 1000 >= expiresAt

    companion object {
        /**
         * 从 token 响应构造（对标 from_token_response）。
         * @param expiresIn 过期秒数（容忍 Int/Float，默认 3600）
         */
        fun fromResponse(
            accessToken: String,
            refreshToken: String?,
            expiresInSec: Long,
            tokenType: String?,
            scope: String?,
            nowMs: Long,
        ): TokenPair = TokenPair(
            accessToken = accessToken,
            refreshToken = refreshToken ?: "",
            expiresAt = nowMs + expiresInSec * 1000,
            tokenType = tokenType ?: "Bearer",
            scope = scope,
        )
    }
}
