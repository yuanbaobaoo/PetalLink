package io.github.yuanbaobaoo.petallink.auth

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.core.logging.Logger
import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * OAuth 授权服务（对标 src/auth/service.rs）。
 *
 * 职责：code→token 交换、token 存取、ensureValid 提前刷新。
 * 注意：换 token 时手工拼 form body（+→%2B），不用 .form()。
 */
class AuthService(
    private val httpClient: HttpClient,
    private val clientIdProvider: () -> String,
    private val clientSecretProvider: () -> String,
    private val tokenStore: TokenStore,
    private val refresher: TokenRefresher,
) {
    private val json = Json { ignoreUnknownKeys = true }
    private val logger = Logger()

    /**
     * 用授权码换 token（对标 exchange_code_for_token）。
     * 踩坑 3：手工拼 form body，code 的 + 必须编码 %2B。
     */
    suspend fun exchangeCodeForToken(
        code: String,
        redirectUri: String,
        pkceVerifier: String?,
    ): TokenPair {
        // 手工拼 form body（enc 编码，踩坑 3：+→%2B）
        val parts = mutableListOf(
            "grant_type" to "authorization_code",
            "code" to Pkce.enc(code),
            "client_id" to Pkce.enc(clientIdProvider()),
            "client_secret" to Pkce.enc(clientSecretProvider()),
            "redirect_uri" to Pkce.enc(redirectUri),
        )
        if (pkceVerifier != null) {
            parts += "code_verifier" to Pkce.enc(pkceVerifier)
        }
        val formBody = parts.joinToString("&") { "${it.first}=${it.second}" }

        val resp = httpClient.request(AuthConstants.TOKEN_URL) {
            method = HttpMethod.Post
            header(HttpHeaders.ContentType, ContentType.Application.FormUrlEncoded.toString())
            setBody(formBody)
        }
        if (resp.status.value != 200) {
            val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
            val desc = body["error_description"]?.jsonPrimitive?.content
                ?: body["error"]?.jsonPrimitive?.content
                ?: "未知错误"
            logger.error("auth.service", { "换 token 失败：$desc status=${resp.status.value}" }, null)
            throw AppError.Auth("授权码换 token 失败: $desc")
        }
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        val accessToken = body["access_token"]?.jsonPrimitive?.content
            ?: throw AppError.Auth("换 token 响应缺少 access_token")
        val refreshToken = body["refresh_token"]?.jsonPrimitive?.content ?: ""
        val expiresIn = body["expires_in"]?.jsonPrimitive?.content?.toLongOrNull() ?: 3600L
        val tokenType = body["token_type"]?.jsonPrimitive?.content ?: "Bearer"
        val scope = body["scope"]?.jsonPrimitive?.content

        val token = TokenPair.fromResponse(
            accessToken = accessToken,
            refreshToken = refreshToken,
            expiresInSec = expiresIn,
            tokenType = tokenType,
            scope = scope,
            nowMs = io.github.yuanbaobaoo.petallink.drive.PlatformTime.millis(),
        )
        tokenStore.save(token)
        return token
    }

    /**
     * 确保 token 有效：临过期则提前刷新（对标 ensure_valid_access_token）。
     * @return 当前有效的 access_token
     */
    suspend fun ensureValidAccessToken(): String {
        val token = tokenStore.load()
            ?: throw AppError.Auth("未登录")
        if (token.willExpireWithin(AuthConstants.TOKEN_EXPIRY_BUFFER_SECS, io.github.yuanbaobaoo.petallink.drive.PlatformTime.millis())) {
            val refreshed = refresher.refresh()
            return refreshed.accessToken
        }
        return token.accessToken
    }
}

/**
 * Token 存储接口（对标 src/auth/token_store.rs TokenStore trait）。
 * actual 由 macosMain 提供（加密文件持久化）。
 */
interface TokenStore {
    /**
     * 读取已持久化的 token，未登录返回 null
     */
    suspend fun load(): TokenPair?

    /**
     * 持久化保存 token
     */
    suspend fun save(token: TokenPair)

    /**
     * 清除已保存的 token（登出）
     */
    suspend fun clear()
}
