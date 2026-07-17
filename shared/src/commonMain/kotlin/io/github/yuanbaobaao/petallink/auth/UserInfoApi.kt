package io.github.yuanbaobaao.petallink.auth

import io.github.yuanbaobaao.petallink.AppError
import io.ktor.client.HttpClient
import io.ktor.client.request.header
import io.ktor.client.request.request
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpMethod
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * 华为账号资料聚合客户端。
 *
 * 三个端点并发请求，单端点失败只丢弃该端点结果。合并顺序固定为
 * OIDC < display info < phone，确保手机号端点提供的真实值优先。
 */
class UserInfoApi(
    private val httpClient: HttpClient,
    private val tokenProvider: suspend () -> String,
    private val restPhpUrl: String = AuthConstants.REST_PHP_URL,
    private val oidcUrl: String = AuthConstants.USER_INFO_URL,
) {
    suspend fun get(): UserInfo = coroutineScope {
        val token = tokenProvider()
        val info = async { runCatching { getDisplayInfo(token) }.getOrNull() }
        val phone = async { runCatching { getPhone(token) }.getOrNull() }
        val oidc = async { runCatching { getOidc(token) }.getOrNull() }

        val merged = linkedMapOf<String, kotlinx.serialization.json.JsonElement>()
        oidc.await()?.let(merged::putAll)
        info.await()?.let(merged::putAll)
        phone.await()?.let(merged::putAll)
        UserInfo.fromJson(JsonObject(merged)).resolveAnonymousAsMobile()
    }

    private suspend fun getDisplayInfo(token: String): JsonObject {
        val body = "access_token=${Pkce.enc(token)}&getNickName=1"
        return requestObject(
            HttpMethod.Post,
            "$restPhpUrl?nsp_svc=GOpen.User.getInfo",
            body,
        )
    }

    private suspend fun getPhone(token: String): JsonObject {
        val response = httpClient.request("$restPhpUrl?nsp_svc=GOpen.User.getPhone") {
            method = HttpMethod.Post
            header(HttpHeaders.ContentType, ContentType.Application.FormUrlEncoded.toString())
            setBody("access_token=${Pkce.enc(token)}")
        }
        if (response.status.value !in 200..299) {
            throw AppError.Remote(response.status.value, "GOpen.User.getPhone 请求失败")
        }
        val text = response.bodyAsText().trim()
        if (text.isEmpty()) return JsonObject(emptyMap())
        val parsed = runCatching { json.parseToJsonElement(text) }.getOrNull()
        if (parsed is JsonObject) return parsed
        return JsonObject(mapOf("mobile" to JsonPrimitive(text)))
    }

    private suspend fun getOidc(token: String): JsonObject = requestObject(
        HttpMethod.Get,
        oidcUrl,
        body = null,
        bearerToken = token,
    )

    private suspend fun requestObject(
        method: HttpMethod,
        url: String,
        body: String?,
        bearerToken: String? = null,
    ): JsonObject {
        val response = httpClient.request(url) {
            this.method = method
            if (bearerToken != null) header(HttpHeaders.Authorization, "Bearer $bearerToken")
            if (body != null) {
                header(HttpHeaders.ContentType, ContentType.Application.FormUrlEncoded.toString())
                setBody(body)
            }
        }
        if (response.status.value !in 200..299) {
            throw AppError.Remote(response.status.value, "用户信息端点请求失败")
        }
        val element = json.parseToJsonElement(response.bodyAsText())
        return element as? JsonObject
            ?: throw AppError.Data("用户信息端点返回非对象 JSON")
    }

    private val json = Json { ignoreUnknownKeys = true; isLenient = true }
}

/** 多端点合并后的完整账号资料。 */
data class UserInfo(
    val sub: String? = null,
    val openId: String? = null,
    val unionId: String? = null,
    val displayName: String? = null,
    val name: String? = null,
    val nickname: String? = null,
    val email: String? = null,
    val mobile: String? = null,
    val avatarUrl: String? = null,
    val isAnonymized: Boolean = false,
) {
    val primaryLabel: String?
        get() = listOf(displayName, mobile, name, nickname, openId, sub)
            .firstNotNullOfOrNull(::nonBlank)

    val secondaryLabel: String?
        get() {
            val primary = primaryLabel
            nonBlank(email)?.takeIf { it != primary }?.let { return it }
            nonBlank(mobile)?.takeIf { it != primary }?.let { return it }
            return if (isAnonymized) "匿名账号" else null
        }

    val initial: String? get() = primaryLabel?.firstOrNull()?.toString()

    fun resolveAnonymousAsMobile(): UserInfo =
        if (isAnonymized && nonBlank(mobile) != null) copy(displayName = null) else this

    companion object {
        fun fromJson(json: JsonObject): UserInfo {
            fun pick(vararg keys: String): String? = keys.firstNotNullOfOrNull { key ->
                json[key]?.let { value ->
                    (value as? JsonPrimitive)?.contentOrNull?.trim()?.takeIf(String::isNotEmpty)
                }
            }
            val flag = json["displayNameFlag"]?.let { value ->
                (value as? JsonPrimitive)?.contentOrNull?.toDoubleOrNull()?.toInt() == 1
            } ?: false
            return UserInfo(
                sub = pick("sub", "user_id", "userId"),
                openId = pick("openID", "openId", "open_id"),
                unionId = pick("unionID", "unionId", "union_id"),
                displayName = pick("displayName", "display_name"),
                name = pick("name"),
                nickname = pick("nickname", "nick_name", "preferred_username"),
                email = pick("email"),
                mobile = pick("mobile", "phone", "phone_number", "mobile_number"),
                avatarUrl = pick("headPictureURL", "picture", "avatar", "avatar_url"),
                isAnonymized = flag,
            )
        }

        private fun nonBlank(value: String?): String? = value?.trim()?.takeIf(String::isNotEmpty)
    }
}
