package io.github.yuanbaobaao.petallink.drive

/** 华为 Files JSON 写接口要求的 ASCII-only 编码。 */
object AsciiJson {
    /** 转义任意原始字符串，保留旧调用面。 */
    fun escape(input: String): String = buildString(input.length) {
        for (char in input) {
            if (char.code in 0x20..0x7e) append(char)
            else append("\\u").append(char.code.toString(16).padStart(4, '0'))
        }
    }

    /**
     * 将已序列化 JSON 中每个非 ASCII UTF-16 code unit 转成 `\\uXXXX`。
     * supplementary code point 会自然变成两个 surrogate escape，与原实现一致。
     */
    fun escapeNonAscii(json: String): String = buildString(json.length) {
        for (char in json) {
            if (char.code <= 0x7f) append(char)
            else append("\\u").append(char.code.toString(16).padStart(4, '0'))
        }
    }
}
