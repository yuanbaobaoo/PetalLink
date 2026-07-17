package io.github.yuanbaobaao.petallink.drive

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse

class AsciiJsonTest {
    @Test
    fun 中文和emoji按UTF16单元转义() {
        val encoded = AsciiJson.escapeNonAscii("{\"fileName\":\"花🌸\"}")
        assertEquals("{\"fileName\":\"\\u82b1\\ud83c\\udf38\"}", encoded)
        assertFalse(encoded.any { it.code > 0x7f })
    }
}
