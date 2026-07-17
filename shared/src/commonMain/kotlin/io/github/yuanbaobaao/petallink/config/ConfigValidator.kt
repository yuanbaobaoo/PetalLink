package io.github.yuanbaobaao.petallink.config

/**
 * 配置校验器（对标 src/core/config.rs validate 规则）。
 *
 * 返回错误消息列表；空列表表示配置合法。
 * 详见 docs/10 阶段 1 item 1。
 */
object ConfigValidator {

    /** 并发数合法范围 */
    const val MIN_CONCURRENCY = 1
    const val MAX_CONCURRENCY = 20
    /** 轮询间隔下限（0 表示禁用，合法） */
    const val MIN_POLL_INTERVAL_SEC = 60L

    /**
     * 校验配置，返回错误列表（空即合法）。
     */
    fun validate(config: UserConfig): List<String> {
        val errors = mutableListOf<String>()

        // 并发数 ∈ [1, 20]
        if (config.concurrency !in MIN_CONCURRENCY..MAX_CONCURRENCY) {
            errors.add("concurrency 必须在 $MIN_CONCURRENCY..$MAX_CONCURRENCY，当前 ${config.concurrency}")
        }

        // 轮询间隔：0（禁用）或 >= 60
        if (config.pollIntervalSec != 0L && config.pollIntervalSec < MIN_POLL_INTERVAL_SEC) {
            errors.add("pollIntervalSec 必须为 0（禁用）或 >= $MIN_POLL_INTERVAL_SEC，当前 ${config.pollIntervalSec}")
        }

        // 去抖 >= 1
        if (config.debounceSec < 1) {
            errors.add("debounceSec 必须 >= 1，当前 ${config.debounceSec}")
        }

        // OAuth 端口 > 0
        if (config.oauthCallbackPort <= 0) {
            errors.add("oauthCallbackPort 必须 > 0，当前 ${config.oauthCallbackPort}")
        }

        // 未完成首次目录选择时允许 mountDir 为空，且绝不能启动同步。
        if (config.mountConfigured) {
            val dir = config.mountDir.trim()
            when {
                dir.isEmpty() -> errors.add("mountDir 不能为空")
                !dir.startsWith("/") && !dir.startsWith("~/") -> errors.add("mountDir 必须是绝对路径或 ~/ 路径")
                dir == "/" -> errors.add("mountDir 不能是根目录 /")
                dir.contains("..") -> errors.add("mountDir 不能包含 .. （防目录穿越）")
                dir.contains("/Library/Application Support") ->
                    errors.add("mountDir 不能位于 Application Support")
            }
        }

        return errors
    }

    /** 便捷方法：配置是否合法 */
    fun isValid(config: UserConfig): Boolean = validate(config).isEmpty()
}
