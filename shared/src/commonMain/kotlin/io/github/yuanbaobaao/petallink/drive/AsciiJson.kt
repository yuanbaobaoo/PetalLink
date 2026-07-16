package io.github.yuanbaobaao.petallink.drive

/**
 * 华为 Drive API 文件名 ASCII 转义（对标原项目 src/drive/ascii_json.rs）
 *
 * 华为 Drive 对部分接口的文件名/路径做了严格 ASCII 校验：非 ASCII 字符必须转义为
 * `\uXXXX`。直接发送中文/emoji 会被服务端拒绝。
 *
 * 算法（docs/03 §ascii_json）：
 * - ASCII 可打印字符（0x20~0x7E）原样输出
 * - 其他 BMP 字符 → `\uXXXX`（4 位十六进制）
 * - 辅助平面字符（U+10000 以上，如部分 emoji）→ 先转 UTF-16 代理对，再拆成两个 `\uXXXX`
 *
 * 注意：kotlinx-serialization-json 默认输出原始 UTF-8；本函数供手工拼接华为 API
 * 请求体/查询参数时使用（docs/03 中那些需要严格 ASCII 的端点）。
 */
object AsciiJson {

    private val HEX = "0123456789abcdef".toCharArray()

    /**
     * 把任意字符串转义为仅含 ASCII 的形式（非 ASCII 字符 → `\uXXXX`）。
     * 供手工拼接华为 API 请求体/查询参数使用。
     */
    fun escape(input: String): String {
        val sb = StringBuilder(input.length + input.length / 3)
        var i = 0

        while (i < input.length) {
            val high = input[i++].code

            // 检测 UTF-16 代理对 → 组合成完整码点
            val codePoint: Int = if (high in 0xD800..0xDBFF && i < input.length) {
                val low = input[i].code
                if (low in 0xDC00..0xDFFF) {
                    i++
                    0x10000 + ((high - 0xD800) shl 10) + (low - 0xDC00)
                } else {
                    high
                }
            } else {
                high
            }

            when {
                // ASCII 可见字符范围原样保留
                codePoint in 0x20..0x7E -> sb.append(codePoint.toChar())

                // 基本多语言平面（BMP）：单個 \uXXXX
                codePoint <= 0xFFFF -> appendUnicode(sb, codePoint)

                // 辅助平面：UTF-16 代理对，拆成两个 \uXXXX
                else -> {
                    val adjusted = codePoint - 0x10000
                    appendUnicode(sb, 0xD800 + (adjusted ushr 10))    // 高代理位
                    appendUnicode(sb, 0xDC00 + (adjusted and 0x3FF))  // 低代理位
                }
            }
        }

        return sb.toString()
    }

    /** 追加单个 `\uXXXX`（4 位小写十六进制） */
    private fun appendUnicode(sb: StringBuilder, code: Int) {
        sb.append("\\u")
        sb.append(HEX[(code ushr 12) and 0xF])
        sb.append(HEX[(code ushr 8) and 0xF])
        sb.append(HEX[(code ushr 4) and 0xF])
        sb.append(HEX[code and 0xF])
    }
}
