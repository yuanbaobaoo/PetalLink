package io.github.yuanbaobaao.petallink.sync.engine

import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * SHA-256 纯 Kotlin 实现单测（用已知测试向量验证正确性）。
 */
class Sha256Test {

    @Test
    fun 空字符串() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assertEquals(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            sha256Pure("".encodeToByteArray()),
        )
    }

    @Test
    fun abc() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assertEquals(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            sha256Pure("abc".encodeToByteArray()),
        )
    }

    @Test
    fun 较长文本() {
        // SHA-256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        assertEquals(
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
            sha256Pure("hello world".encodeToByteArray()),
        )
    }
}
