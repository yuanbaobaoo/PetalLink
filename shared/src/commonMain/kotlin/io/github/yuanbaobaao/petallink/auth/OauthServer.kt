package io.github.yuanbaobaao.petallink.auth

import io.github.yuanbaobaao.petallink.AppError
import kotlinx.coroutines.TimeoutCancellationException


import kotlinx.coroutines.withTimeout
import java.net.ServerSocket
import java.net.URLDecoder
import kotlin.text.Charsets

/**
 * OAuth 回调 HTTP 服务器（对标 src/auth/oauth_server.rs）。
 *
 * 绑定 127.0.0.1:{port}，单次接受，解析 query 参数后自动停止。
 * 详见 docs/07 §OAuth。
 */
class OauthServer(private val port: Int) {
    private var listener: java.net.ServerSocket? = null

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
        listener = java.net.ServerSocket(port, 1, java.net.InetAddress.getByName(AuthConstants.LOOPBACK_HOST))
        val server = listener ?: throw AppError.Network("无法绑定 OAuth 回调端口 $port")

        return try {
            withTimeout(AuthConstants.OAUTH_TIMEOUT_SECS * 1000L) {
                val socket = server.accept()
                val input = socket.getInputStream().bufferedReader()
                val requestLine = input.readLine() ?: throw AppError.Remote(0, "空请求")

                // 解析 GET /oauth/callback?code=...&state=... HTTP/1.1
                val parts = requestLine.split(" ")
                if (parts.size < 2 || !parts[1].startsWith(AuthConstants.CALLBACK_PATH)) {
                    throw AppError.Remote(0, "非法回调请求: $requestLine")
                }
                val queryIdx = parts[1].indexOf('?')
                val queryStr = if (queryIdx >= 0) parts[1].substring(queryIdx + 1) else ""

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
            throw e
        }
    }

    /**
     * 停止服务器（主动取消）。
     */
    fun stop() {
        try { listener?.close() } catch (e: Throwable) {}
    }

    /** 解析 query string */
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

    /** URL 解码（form-urlencoded：+ → 空格，%XX → 字节） */
    private fun urlDecode(s: String): String {
        return s.replace('+', ' ').let {
            URLDecoder.decode(it, "UTF-8")
        }
    }

    /** 生成回调响应页面 */
    private fun buildResponsePage(result: Map<String, String>): String {
        val css = "body{font-family:-apple-system,BlinkMacSystemFont,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f5f5f5}div{text-align:center;padding:40px;border-radius:12px;background:#fff;box-shadow:0 2px 8px rgba(0,0,0,0.1)}h2{color:#333}p{color:#666}"
        val title = if (result["code"] != null) "登录成功" else "登录失败"
        val msg = result["error_description"] ?: result["error"] ?: "请返回应用"
        return "<!DOCTYPE html><html><head><meta charset='UTF-8'><title>PetalLink</title><style>$css</style></head><body><div><h2>$title</h2><p>$msg</p></div></body></html>"
    }
}
