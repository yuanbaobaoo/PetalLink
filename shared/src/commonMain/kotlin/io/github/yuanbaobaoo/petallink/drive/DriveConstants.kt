package io.github.yuanbaobaoo.petallink.drive

/**
 * 华为 Drive API 端点与协议常量（对标原项目 src/drive/ 目录）
 *
 * 详见 docs/03-华为Drive-API接口规范.md（18 条踩坑）。
 */
object DriveConstants {

    // ------------------------------------------------------------------
    // OAuth / 授权（docs/07 §2）
    // ------------------------------------------------------------------

    /**
     * OAuth 授权端点
     */
    const val OAUTH_AUTHORIZE_URL = "https://oauth.cloud.huawei.com/rest.php"

    /**
     * Token 端点（换 token / 刷新 token）
     */
    const val OAUTH_TOKEN_URL = "https://oauth.cloud.huawei.com/rest.php"

    /**
     * scope 中的 `/` 不编码（华为特例，docs/03 踩坑 1）
     */
    const val OAUTH_SCOPE = "https://www.huawei.com/auth/drive/file"

    // ------------------------------------------------------------------
    // Drive API 基址
    // ------------------------------------------------------------------

    /**
     * Drive 文件操作 API 基址
     */
    const val DRIVE_API_BASE = "https://drive.cloud.huawei.com.cn/drive/v1"

    // ------------------------------------------------------------------
    // 关键协议细节（docs/03）
    // ------------------------------------------------------------------

    /**
     * 授权码换 token 时，手工拼 form body。
     * 编码函数必须把 `+` 转为 `%2B`（docs/03 踩坑 2）。
     * 不能用 Ktor 默认的 .form()，它会遗漏此场景。
     */
    const val AUTH_CODE_PLUS_ENCODING = "%2B"

    /**
     * 分块上传的 Content-Type 是 multipart/related（不是 form-data）。
     */
    const val MULTIPART_RELATED = "multipart/related"

    /**
     * 上传恢复：308 响应的 Range/rangeList 需做连续性校验。
     * parse_confirmed_offset 算法确认服务端已接收的偏移量。
     */
    const val RESUMABLE_STATUS_PARTIAL_CONTENT = 308

    /**
     * 增量 changes 翻页：
     * - nextCursor：中间页游标
     * - newStartCursor：终页游标（语义不同，不能混用）
     */
    const val CHANGES_PARAM_NEXT_CURSOR = "nextCursor"
    const val CHANGES_PARAM_NEW_START_CURSOR = "newStartCursor"

    /**
     * 三种删除信号（docs/03 changes 章节）：deleted / trashDone / recycled
     */
    val DELETE_SIGNALS = setOf("deleted", "trashDone", "recycled")
}
