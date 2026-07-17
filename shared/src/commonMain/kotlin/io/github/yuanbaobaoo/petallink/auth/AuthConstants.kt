package io.github.yuanbaobaoo.petallink.auth

/**
 * OAuth 与端点常量（对标原项目 src/core/constants.rs + auth/）
 *
 * 详见 docs/03、docs/07。
 */
object AuthConstants {
    /** 授权端点 */
    const val AUTHORIZE_URL = "https://oauth-login.cloud.huawei.com/oauth2/v3/authorize"
    /** Token 端点（换 token / 刷新 token） */
    const val TOKEN_URL = "https://oauth-login.cloud.huawei.com/oauth2/v3/token"
    /** OIDC 用户信息端点（常 404，静默跳过） */
    const val USER_INFO_URL = "https://oauth-login.cloud.huawei.com/oauth2/v3/userinfo"
    /** 华为账号 REST 端点（getInfo / getPhone） */
    const val REST_PHP_URL = "https://account.cloud.huawei.com/rest.php"

    /** 回环主机（绝不 0.0.0.0） */
    const val LOOPBACK_HOST = "127.0.0.1"
    /** OAuth 回调路径 */
    const val CALLBACK_PATH = "/oauth/callback"
    /** OAuth 回调等待超时：5 分钟 */
    const val OAUTH_TIMEOUT_SECS = 300
    /** token 临近过期的提前刷新窗口：60 秒 */
    const val TOKEN_EXPIRY_BUFFER_SECS = 60L

    /** OAuth scope 列表 */
    val SCOPES = listOf("openid", "profile", "https://www.huawei.com/auth/drive")

    /** token.bin 魔数 */
    val MAGIC: ByteArray = byteArrayOf('P'.code.toByte(), 'T'.code.toByte(), 'L'.code.toByte(), '1'.code.toByte())
    /** ChaCha20-Poly1305 nonce 长度 */
    const val NONCE_LEN = 12
    /** Poly1305 tag 长度 */
    const val TAG_LEN = 16
}
