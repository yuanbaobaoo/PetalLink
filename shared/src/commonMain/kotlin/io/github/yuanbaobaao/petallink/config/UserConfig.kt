package io.github.yuanbaobaao.petallink.config

import kotlinx.serialization.Serializable

/**
 * 用户可配置项（对标 src/core/config.rs）。
 *
 * 通过 [ConfigStore] 持久化为 JSON 文件。所有默认值与 AppConfig 常量一致。
 */
@Serializable
data class UserConfig(
    /** 挂载目录（绝对路径，`~` 展开为 home） */
    val mountDir: String = "",
    /** 并发传输数（默认 6，范围 [1, 20]） */
    val concurrency: Int = 6,
    /** 增量轮询间隔秒（默认 60；0 表示禁用轮询，>=60 合法） */
    val pollIntervalSec: Long = 60L,
    /** 文件监听去抖秒（默认 3，>=1） */
    val debounceSec: Long = 3L,
    /** OAuth 回调监听端口（>0） */
    val oauthCallbackPort: Int = 17890,
)
