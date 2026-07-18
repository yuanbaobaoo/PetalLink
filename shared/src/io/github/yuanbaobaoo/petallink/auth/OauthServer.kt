package io.github.yuanbaobaoo.petallink.auth

import io.github.yuanbaobaoo.petallink.AppError
import kotlinx.coroutines.TimeoutCancellationException


import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketTimeoutException
import java.net.URLDecoder
import kotlin.text.Charsets

/**
 * OAuth 回调 HTTP 服务器（对标 src/auth/oauth_server.rs）。
 *
 * 绑定 127.0.0.1:{port}，单次接受，解析 query 参数后自动停止。
 * 详见 docs/07 §OAuth。
 */
class OauthServer(
    private val port: Int,
    private val timeoutMs: Long = AuthConstants.OAUTH_TIMEOUT_SECS * 1000L,
) {
    @Volatile private var listener: java.net.ServerSocket? = null

    /**
     * 先绑定端口，再由调用方打开浏览器，避免快速回调竞态。
     */
    fun bind() {
        if (listener != null) return
        listener = java.net.ServerSocket(port, 1, java.net.InetAddress.getByName(AuthConstants.LOOPBACK_HOST)).also {
            it.soTimeout = 100
        }
    }

    /**
     * OAuth 回调结果，包含授权码、state 及错误信息
     */
    data class CallbackResult(
        val code: String?,
        val state: String?,
        val error: String?,
        val errorDescription: String?,
    )

    /**
     * 启动服务器并等待回调（5 分钟超时）。
     * @return 回调结果（code/state/error）
     */
    suspend fun waitForCallback(): CallbackResult {
        bind()
        val server = listener ?: throw AppError.Network("无法绑定 OAuth 回调端口 $port")

        return try {
            withTimeout(timeoutMs) {
                val socket = acceptCancellable(server)
                val input = socket.getInputStream().bufferedReader()
                val requestLine = input.readLine() ?: throw AppError.Remote(0, "空请求")

                // 解析 GET /oauth/callback?code=...&state=... HTTP/1.1
                val parts = requestLine.split(" ")
                val requestTarget = parts.getOrNull(1).orEmpty()
                if (parts.size < 2 || requestTarget.substringBefore('?') != AuthConstants.CALLBACK_PATH) {
                    throw AppError.Remote(0, "非法回调请求: $requestLine")
                }
                val queryIdx = requestTarget.indexOf('?')
                val queryStr = if (queryIdx >= 0) requestTarget.substring(queryIdx + 1) else ""

                val params = parseQuery(queryStr)

                // 返回 HTML 响应
                val responseHtml = buildResponsePage(params)
                val bytes = responseHtml.toByteArray(Charsets.UTF_8)
                val output = socket.getOutputStream()
                output.write("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: ${bytes.size}\r\nConnection: close\r\n\r\n".toByteArray())
                output.write(bytes)
                output.flush()
                socket.close()
                server.close()

                CallbackResult(
                    code = params["code"],
                    state = params["state"],
                    error = params["error"],
                    errorDescription = params["error_description"],
                )
            }
        } catch (e: TimeoutCancellationException) {
            server.close()
            throw AppError.Auth("OAuth 回调超时（${AuthConstants.OAUTH_TIMEOUT_SECS}秒）")
        } catch (e: Throwable) {
            server.close()
            if (listener == null) throw AppError.Canceled("OAuth 登录已取消")
            throw e
        }
    }

    /**
     * 可取消地等待连接：SO_TIMEOUT 触发时 yield 让出协程，支持外部 stop
     */
    private suspend fun acceptCancellable(server: ServerSocket): Socket {
        while (true) {
            try {
                return server.accept()
            } catch (_: SocketTimeoutException) {
                yield()
            }
        }
    }

    /**
     * 停止服务器（主动取消）。
     */
    fun stop() {
        try { listener?.close() } catch (e: Throwable) {}
        listener = null
    }

    /**
     * 解析 query string
     */
    private fun parseQuery(query: String): Map<String, String> {
        if (query.isEmpty()) return emptyMap()
        val map = mutableMapOf<String, String>()
        for (pair in query.split("&")) {
            val eqIdx = pair.indexOf('=')
            if (eqIdx < 0) continue
            val key = urlDecode(pair.substring(0, eqIdx))
            val value = urlDecode(pair.substring(eqIdx + 1))
            map[key] = value
        }
        return map
    }

    /**
     * URL 解码（form-urlencoded：+ → 空格，%XX → 字节）
     */
    private fun urlDecode(s: String): String {
        return s.replace('+', ' ').let {
            URLDecoder.decode(it, "UTF-8")
        }
    }

    /**
     * 生成回调响应页面
     */
    private fun buildResponsePage(result: Map<String, String>): String {
        val css = "body{font-family:-apple-system,BlinkMacSystemFont,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f5f5f5}div{text-align:center;padding:40px;border-radius:12px;background:#fff;box-shadow:0 2px 8px rgba(0,0,0,0.1)}h2{color:#333}p{color:#666}"
        val title = if (result["code"] != null) "登录成功" else "登录失败"
        val msg = result["error_description"] ?: result["error"] ?: "请返回应用"
        return "<!DOCTYPE html><html><head><meta charset='UTF-8'><title>PetalLink</title><style>$css</style></head><body><div><h2>$title</h2><p>$msg</p></div></body></html>"
    }
}

/**
 * OAuth 回调校验器，校验 state 并提取授权码或抛出鉴权异常
 */
object OauthCallbackValidator {
    /**
     * 校验 state 匹配并提取授权码，否则抛出鉴权异常
     */
    fun requireCode(result: OauthServer.CallbackResult, expectedState: String): String {
        if (result.state != expectedState) throw AppError.Auth("OAuth state 校验失败")
        return result.code
            ?: throw AppError.Auth("授权失败: ${result.errorDescription ?: result.error ?: "未知"}")
    }
}
