package io.github.yuanbaobaao.petallink.drive

import io.github.yuanbaobaao.petallink.auth.TokenPair
import io.ktor.client.*
import io.ktor.client.plugins.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*

/**
 * 华为 Drive HTTP 客户端配置（对标 src/drive/client.rs）。
 *
 * 详见 docs/03 §HTTP 客户端、docs/10 阶段 2 item 7。
 */
object DriveClientConfig {
    /** 普通 API 超时：连接 15s，请求 60s */
    const val CONNECT_TIMEOUT_MS = 15_000L
    const val REQUEST_TIMEOUT_MS = 60_000L
    /** 上传 API 超时：120s（大文件） */
    const val UPLOAD_TIMEOUT_MS = 120_000L
    /** 每 host 最大空闲连接 */
    const val MAX_CONNECTIONS_PER_ROUTE = 15

    /** 判定是否为 token 端点（跳过 Bearer 注入，防刷新死循环） */
    fun isTokenEndpoint(url: String): Boolean = url.contains("oauth2/v3/token")
}

/**
 * HTTP 请求语义（Read/Write）。
 */
enum class HttpSemantics { READ, WRITE }

/**
 * Drive HTTP 客户端封装。
 *
 * 职责（对标 src/drive/client.rs execute_with_retry）：
 * - Bearer 注入（token 端点除外）
 * - 401 自动刷新重放（仅一次）
 * - 错误分类与状态码处理
 *
 * @param httpClient Ktor 客户端（由平台提供，darwin engine）
 * @param tokenProvider 当前 token 获取（含 ensureValid 提前刷新）
 * @param tokenRefresher 401 时触发刷新
 */
class DriveClient(
    private val httpClient: HttpClient,
    private val tokenProvider: suspend () -> String,
    private val tokenRefresher: suspend () -> TokenPair,
) {
    /**
     * 执行带 401 重放的请求（对标 execute_with_retry）。
     *
     * @param method HTTP 方法
     * @param url 完整 URL
     * @param semantics 读/写语义
     * @param configure 请求构建器（body/headers 等）
     * @return HTTP 响应
     */
    suspend fun executeWithRetry(
        method: HttpMethod,
        url: String,
        semantics: HttpSemantics,
        configure: HttpRequestBuilder.() -> Unit = {},
    ): HttpResponse {
        val skipAuth = DriveClientConfig.isTokenEndpoint(url)
        val token = if (skipAuth) null else tokenProvider()

        // 第一次请求
        val resp1 = sendRequest(method, url, token, configure)
        if (resp1.status.value != 401) return resp1

        // 401 → 刷新 token 后重放一次
        val refreshed = tokenRefresher()
        val resp2 = sendRequest(method, url, refreshed.accessToken, configure)
        return resp2
    }

    /** 发送单次请求（注入 Bearer） */
    private suspend fun sendRequest(
        method: HttpMethod,
        url: String,
        token: String?,
        configure: HttpRequestBuilder.() -> Unit,
    ): HttpResponse = httpClient.request(url) {
        this.method = method
        if (token != null) {
            header(HttpHeaders.Authorization, "Bearer $token")
        }
        configure()
    }

    companion object {
        /** 构建 Ktor HttpClient 配置（普通 API） */
        fun defaultConfig(): HttpClientConfig<*>.() -> Unit = {
            install(HttpTimeout) {
                connectTimeoutMillis = DriveClientConfig.CONNECT_TIMEOUT_MS
                requestTimeoutMillis = DriveClientConfig.REQUEST_TIMEOUT_MS
            }
        }

        /** 构建上传专用 HttpClient 配置（更长超时 + 禁用自动重定向） */
        fun uploadConfig(): HttpClientConfig<*>.() -> Unit = {
            install(HttpTimeout) {
                connectTimeoutMillis = DriveClientConfig.CONNECT_TIMEOUT_MS
                requestTimeoutMillis = DriveClientConfig.UPLOAD_TIMEOUT_MS
            }
            followRedirects = false  // 308/Location 不能被自动跟随
        }
    }
}
