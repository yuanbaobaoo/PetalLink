package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.config.AppConfig

/**
 * 文件跳过过滤器（对标 src/mount/skip.rs）。
 *
 * 详见 docs/06 §内部文件隔离、docs/10 阶段 3 item 17。
 * 规则：
 * - `.hwcloud_` 前缀全局硬编码过滤（绝不参与同步）
 * - `.tmp` 后缀（下载临时文件）
 * - 旧版 `.hwcloud_placeholder`
 * - 用户自定义 glob 跳过模式（默认 .DS_Store / .tmp / ~$* / .Trash）
 */
object SkipFilter {

    /** 内部文件前缀（硬编码，全局生效） */
    const val INTERNAL_PREFIX = ".hwcloud_"

    /** 下载临时文件后缀 */
    const val TMP_SUFFIX = ".tmp"

    /** 旧版占位符文件名 */
    const val LEGACY_PLACEHOLDER = ".hwcloud_placeholder"

    /** 默认 glob 跳过模式 */
    val DEFAULT_PATTERNS: List<String> = listOf(
        ".DS_Store",
        "*.tmp",
        "~$*",
        ".Trash",
    )

    /**
     * 判定文件是否应被跳过（不参与同步）。
     *
     * @param name 文件名（不含路径）
     * @param patterns 用户自定义 glob 模式（默认用 [DEFAULT_PATTERNS]）
     * @return true 表示跳过
     */
    fun shouldSkip(name: String, patterns: List<String> = DEFAULT_PATTERNS): Boolean {
        // 硬编码规则（最高优先级）
        if (name.startsWith(INTERNAL_PREFIX)) return true
        if (name == LEGACY_PLACEHOLDER) return true
        if (name.endsWith(TMP_SUFFIX)) return true

        // glob 模式匹配
        for (pattern in patterns) {
            if (globMatch(pattern, name)) return true
        }
        return false
    }

    /**
     * 简易 glob 匹配（对标原项目手写 glob→regex）。
     * 支持：* 任意字符序列、? 单字符、其他字面量。
     */
    fun globMatch(pattern: String, name: String): Boolean {
        // 把 glob 转为正则：* → .*, ? → ., 其余转义
        val regex = buildString {
            append('^')
            for (c in pattern) {
                when (c) {
                    '*' -> append(".*")
                    '?' -> append('.')
                    // 正则元字符转义
                    in ".+()[]{}^$|\\" -> {
                        append('\\'); append(c)
                    }
                    else -> append(c)
                }
            }
            append('$')
        }.toRegex()
        return regex.matches(name)
    }
}
