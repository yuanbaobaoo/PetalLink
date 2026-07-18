package io.github.yuanbaobaoo.petallink.auth

import io.github.yuanbaobaoo.petallink.AppError
import io.ktor.client.*
import io.ktor.client.request.*
import io.ktor.client.request.forms.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * Token 刷新器（对标 src/auth/token_refresher.rs TokenRefresher + Singleflight）。
 *
 * Singleflight：并发刷新请求共享一次刷新结果（leader/follower 模式）。
 * leader 执行刷新，follower await leader 的 CompletableDeferred。
 */
class TokenRefresher(
    private val httpClient: HttpClient,
    private val clientIdProvider: () -> String,
    private val clientSecretProvider: () -> String,
    private val currentTokenProvider: () -> TokenPair?,
    private val onTokenSaved: (TokenPair) -> Unit,
) {
    private val singleflightMutex = Mutex()
    private var activeFlight: CompletableDeferred<TokenPair>? = null

    /**
     * 执行 token 刷新（Singleflight 去重）。
     * @return 新的 TokenPair
     */
    suspend fun refresh(): TokenPair {
        val (flight, isLeader) = singleflightMutex.withLock {
            val existing = activeFlight
            if (existing != null) existing to false
            else CompletableDeferred<TokenPair>().also { activeFlight = it } to true
        }
        if (!isLeader) return flight.await()

        return try {
            val result = doRefresh()
            onTokenSaved(result)
            flight.complete(result)
            result
        } catch (e: Throwable) {
            flight.completeExceptionally(e)
            throw e
        } finally {
            singleflightMutex.withLock {
                if (activeFlight === flight) activeFlight = null
            }
        }
    }

    /**
     * 实际刷新请求（对标 refresh，用 .form()，refresh_token 无特殊字符）
     */
    private suspend fun doRefresh(): TokenPair {
        val current = currentTokenProvider()
            ?: throw AppError.Auth("无 token 可刷新，需重新登录")
        val params = ParametersBuilder().apply {
            append("grant_type", "refresh_token")
            append("refresh_token", current.refreshToken)
            append("client_id", clientIdProvider())
            append("client_secret", clientSecretProvider())
        }.build()

        val resp = httpClient.submitForm(
            url = AuthConstants.TOKEN_URL,
            formParameters = params,
        )
        if (resp.status.value != 200) {
            throw AppError.Auth("token 刷新失败: ${resp.status.value}")
        }
        val body = Json.parseToJsonElement(resp.bodyAsText()).jsonObject
        val accessToken = body["access_token"]?.jsonPrimitive?.content
            ?: throw AppError.Auth("刷新响应缺少 access_token")
        // refresh_token 可能缺失 → 复用旧的
        val refreshToken = body["refresh_token"]?.jsonPrimitive?.content
            ?: current.refreshToken
        val expiresIn = body["expires_in"]?.jsonPrimitive?.content?.toLongOrNull() ?: 3600L
        val tokenType = body["token_type"]?.jsonPrimitive?.content ?: "Bearer"
        val scope = body["scope"]?.jsonPrimitive?.content ?: current.scope

        return TokenPair.fromResponse(
            accessToken = accessToken,
            refreshToken = refreshToken,
            expiresInSec = expiresIn,
            tokenType = tokenType,
            scope = scope,
            nowMs = io.github.yuanbaobaoo.petallink.drive.PlatformTime.millis(),
        )
    }
}
