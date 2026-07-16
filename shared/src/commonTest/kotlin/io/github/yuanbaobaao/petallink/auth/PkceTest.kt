package io.github.yuanbaobaao.petallink.auth

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * PKCE 授权 URL 与 enc 编码单测（对标 docs/03 §2 踩坑 2、3）。
 */
class PkceTest {

    @Test
    fun enc_加号必须编码为百分号2B() {
        // 踩坑 3：授权码含 +，form-urlencoded 把 + 当空格 → 1101 invalid code
        assertEquals("%2B", Pkce.enc("+"))
    }

    @Test
    fun enc_斜杠必须编码() {
        assertEquals("%2F", Pkce.enc("/"))
    }

    @Test
    fun enc_字母数字不编码() {
        assertEquals("abc123", Pkce.enc("abc123"))
    }

    @Test
    fun enc_unreserved符号不编码() {
        assertEquals("-_.~", Pkce.enc("-_.~"))
    }

    @Test
    fun enc_空格编码为百分号20() {
        assertEquals("%20", Pkce.enc(" "))
    }

    @Test
    fun enc_中文字符UTF8逐字节编码() {
        // "中" = E4 B8 AD
        assertEquals("%E4%B8%AD", Pkce.enc("中"))
    }

    @Test
    fun buildAuthorizeUrl_参数顺序固定且scope斜杠不编码() {
        val pkce = PkcePair(
            codeVerifier = "verifier123",
            codeChallenge = "challenge456",
        )
        val url = Pkce.buildAuthorizeUrl(
            redirectUri = "http://127.0.0.1:9999/oauth/callback",
            state = "abcdef0123456789",
            pkce = pkce,
            clientId = "client123",
        )
        // 验证端点
        assertTrue(url.startsWith(AuthConstants.AUTHORIZE_URL + "?"))
        // 验证参数顺序
        val params = url.substringAfter("?")
        assertTrue(params.startsWith("response_type=code&client_id=client123&"))
        // scope 的 / 不编码（踩坑 2）
        assertTrue(url.contains("scope=openid%20profile%20https://www.huawei.com/auth/drive"))
        // code_challenge_method=S256
        assertTrue(url.contains("code_challenge_method=S256"))
        // access_type=offline
        assertTrue(url.contains("access_type=offline"))
    }

    @Test
    fun buildRedirectUri_格式正确() {
        assertEquals("http://127.0.0.1:9999/oauth/callback", Pkce.buildRedirectUri(9999))
    }
}
