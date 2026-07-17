package io.github.yuanbaobaao.petallink.auth

/**
 * PKCE 与授权 URL 构建（对标原项目 auth/pkce.rs + auth/service.rs）
 *
 * 详见 docs/03 §2、docs/07 §PKCE。
 */

/** PKCE 密钥对 */
data class PkcePair(
    val codeVerifier: String,    // ~86 字符（64 字节 base64url 无填充）
    val codeChallenge: String,   // 43 字符（SHA256 后 base64url 无填充）
)

/**
 * 授权相关纯逻辑（无 IO，可单测）。
 */
object Pkce {
    /** 64 随机字节 verifier + SHA-256 challenge。 */
    fun generate(): PkcePair {
        val verifierBytes = ByteArray(64).also(java.security.SecureRandom()::nextBytes)
        val verifier = base64Url(verifierBytes)
        val challenge = base64Url(java.security.MessageDigest.getInstance("SHA-256").digest(verifier.encodeToByteArray()))
        return PkcePair(verifier, challenge)
    }

    /** OAuth CSRF state：32 字节密码学随机数。 */
    fun generateState(): String = base64Url(ByteArray(32).also(java.security.SecureRandom()::nextBytes))

    private fun base64Url(bytes: ByteArray): String =
        java.util.Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)

    /**
     * 构建授权 URL（参数顺序固定，docs/03 踩坑 2）。
     *
     * 关键：除 scope 外所有参数用 [enc] 编码（RFC3986 unreserved）；
     * scope 的 `/` 不编码（踩坑 2），空格→%20。
     */
    fun buildAuthorizeUrl(
        redirectUri: String,
        state: String,
        pkce: PkcePair,
        clientId: String,
    ): String {
        val scopeRaw = AuthConstants.SCOPES.joinToString(" ").replace(" ", "%20")
        // 注意顺序固定且 load-bearing
        return buildString {
            append(AuthConstants.AUTHORIZE_URL)
            append("?response_type=code")
            append("&client_id=").append(enc(clientId))
            append("&redirect_uri=").append(enc(redirectUri))
            append("&state=").append(enc(state))
            append("&access_type=offline")
            append("&code_challenge=").append(enc(pkce.codeChallenge))
            append("&code_challenge_method=S256")
            append("&scope=").append(scopeRaw)
        }
    }

    /** 构建回调 URI：http://127.0.0.1:{port}/oauth/callback */
    fun buildRedirectUri(port: Int): String =
        "http://${AuthConstants.LOOPBACK_HOST}:$port${AuthConstants.CALLBACK_PATH}"

    /**
     * RFC3986 百分号编码（对标原项目 enc() / urlencoding()）。
     * 仅 A-Za-z0-9-_.~ 不编码，其余全部转义。
     * 关键：`+` → %2B（踩坑 3，授权码含 + 时必须编码）。
     */
    fun enc(s: String): String {
        val sb = StringBuilder(s.length * 2)
        for (b in s.encodeToByteArray()) {
            val v = b.toInt() and 0xFF
            if (IS_UNRESERVED[v]) {
                sb.append(v.toChar())
            } else {
                sb.append('%')
                sb.append(HEX[v ushr 4])
                sb.append(HEX[v and 0xF])
            }
        }
        return sb.toString()
    }

    private val HEX = "0123456789ABCDEF".toCharArray()
    // RFC3986 unreserved: A-Za-z0-9-_.~（查表：true 表示该字节值不编码）
    private val IS_UNRESERVED: BooleanArray = run {
        val set = BooleanArray(256)
        for (c in 'A'..'Z') set[c.code] = true
        for (c in 'a'..'z') set[c.code] = true
        for (c in '0'..'9') set[c.code] = true
        for (c in charArrayOf('-', '_', '.', '~')) set[c.code] = true
        set
    }
}
