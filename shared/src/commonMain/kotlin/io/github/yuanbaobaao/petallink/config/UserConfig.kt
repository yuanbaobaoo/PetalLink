package io.github.yuanbaobaao.petallink.config

import kotlinx.serialization.Serializable
import kotlinx.serialization.SerialName

@Serializable
enum class SortField {
    @SerialName("name") Name,
    @SerialName("size") Size,
    @SerialName("modifiedTime") ModifiedTime,
}

@Serializable
enum class SortOrder {
    @SerialName("ascending") Ascending,
    @SerialName("descending") Descending,
}

/**
 * 用户可配置项（对标 src/core/config.rs）。
 *
 * 通过 [ConfigStore] 持久化为 JSON 文件。所有默认值与 AppConfig 常量一致。
 */
@Serializable
data class UserConfig(
    /** OAuth 回调 URI，必须与 AGC 后台配置一致 */
    val oauthRedirectUri: String = DEFAULT_REDIRECT_URI,
    /** OAuth 回调监听端口（>0） */
    val oauthCallbackPort: Int = DEFAULT_CALLBACK_PORT,
    /** 挂载目录（绝对路径，`~` 展开为 home） */
    val mountDir: String = "",
    /** 用户是否已明确选择过挂载目录；false 时不会启动同步 */
    val mountConfigured: Boolean = false,
    /** 并发传输数（默认 6，范围 [1, 20]） */
    val concurrency: Int = 6,
    /** 增量轮询间隔秒（默认 60；0 表示禁用轮询，>=60 合法） */
    val pollIntervalSec: Long = 60L,
    /** 文件监听去抖秒（默认 3，>=1） */
    val debounceSec: Long = 3L,
    /** 文件名跳过规则 */
    val skipPatterns: List<String> = DEFAULT_SKIP_PATTERNS,
    /** 文件列表排序 */
    val sortField: SortField = SortField.Name,
    val sortOrder: SortOrder = SortOrder.Ascending,
)

const val DEFAULT_REDIRECT_URI = "http://127.0.0.1:9999/oauth/callback"
const val DEFAULT_CALLBACK_PORT = 9999
val DEFAULT_SKIP_PATTERNS = listOf(".DS_Store", ".tmp", "~$*", ".Trash")
