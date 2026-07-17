package io.github.yuanbaobaoo.petallink.auth

import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * TokenSerializer 小端布局单测（对标 docs/07 §token.bin）。
 */
class TokenSerializerTest {

    @Test
    fun 序列化与反序列化往返一致() {
        val token = TokenPair(
            accessToken = "access-xyz",
            refreshToken = "refresh-abc",
            expiresAt = 1700000000000L,
            tokenType = "Bearer",
            scope = "openid profile",
        )
        val bytes = TokenSerializer.serialize(token)
        val restored = TokenSerializer.deserialize(bytes)
        assertEquals(token, restored)
    }

    @Test
    fun 无scope时往返一致() {
        val token = TokenPair(
            accessToken = "at",
            refreshToken = "rt",
            expiresAt = 0L,
            tokenType = "Bearer",
            scope = null,
        )
        val restored = TokenSerializer.deserialize(TokenSerializer.serialize(token))
        assertEquals(token, restored)
    }

    @Test
    fun tokenType用u32前缀而非u64() {
        // 验证 token_type 长度前缀是 4 字节（u32），不是 8 字节（u64）
        val token = TokenPair("a", "b", 0L, "Be", null)
        val bytes = TokenSerializer.serialize(token)
        // 布局：at(8+1) + rt(8+1) + expires(8) + tt_len(4) + tt(2) + scope_present(1)
        // = 9 + 9 + 8 + 4 + 2 + 1 = 33
        assertEquals(33, bytes.size)
    }

    @Test
    fun expiresAt为负数也能往返() {
        val token = TokenPair("a", "b", -1L, "Bearer", null)
        val restored = TokenSerializer.deserialize(TokenSerializer.serialize(token))
        assertEquals(-1L, restored.expiresAt)
    }
}
