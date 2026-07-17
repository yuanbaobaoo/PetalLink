package io.github.yuanbaobaoo.petallink.auth

/**
 * token.bin 明文序列化（对标原项目 auth/token_store.rs serialize/deserialize）
 *
 * 明文布局（**小端**）：
 * - [u64 LE] access_token.len + bytes
 * - [u64 LE] refresh_token.len + bytes
 * - [i64 LE] expires_at（毫秒）
 * - [u32 LE] token_type.len + bytes  ← 注意用 u32（非 u64）
 * - [u8] scope_present（1/0）
 *   - 若 1：[u64 LE] scope.len + bytes
 *
 * 详见 docs/07 §token.bin。
 */
object TokenSerializer {

    /**
     * 序列化为明文字节（加密前）
     */
    fun serialize(token: TokenPair): ByteArray {
        val parts = mutableListOf<ByteArray>()
        // access_token（u64 前缀）
        val at = token.accessToken.encodeToByteArray()
        parts += uLongLe(at.size.toLong()) + at
        // refresh_token（u64 前缀）
        val rt = token.refreshToken.encodeToByteArray()
        parts += uLongLe(rt.size.toLong()) + rt
        // expires_at（i64）
        parts += uLongLe(token.expiresAt)
        // token_type（u32 前缀，注意非 u64）
        val tt = token.tokenType.encodeToByteArray()
        parts += uIntLe(tt.size) + tt
        // scope（u8 present + 可选 u64 前缀 + bytes）
        val scopeBytes = token.scope?.encodeToByteArray()
        if (scopeBytes != null) {
            parts += byteArrayOf(1) + uLongLe(scopeBytes.size.toLong()) + scopeBytes
        } else {
            parts += byteArrayOf(0)
        }
        return parts.reduce { acc, b -> acc + b }
    }

    /**
     * 从明文字节反序列化（解密后）
     */
    fun deserialize(data: ByteArray): TokenPair {
        var pos = 0
        // access_token
        val (at, p1) = readLengthPrefixedU64(data, pos); pos = p1
        // refresh_token
        val (rt, p2) = readLengthPrefixedU64(data, pos); pos = p2
        // expires_at（i64 LE）
        val expiresAt = readULongLe(data, pos); pos += 8
        // token_type（u32 前缀）
        val (tt, p3) = readLengthPrefixedU32(data, pos); pos = p3
        // scope
        val present = data[pos]; pos += 1
        val scope: String? = if (present == 1.toByte()) {
            val (sc, p4) = readLengthPrefixedU64(data, pos); pos = p4
            sc
        } else null

        return TokenPair(
            accessToken = at,
            refreshToken = rt,
            expiresAt = expiresAt,
            tokenType = if (tt.isEmpty()) "Bearer" else tt,
            scope = scope,
        )
    }

    // --- 小端编码/解码辅助 ---

    /**
     * 将 Long 编码为 8 字节小端
     */
    private fun uLongLe(v: Long): ByteArray = ByteArray(8) { i ->
        ((v ushr (i * 8)) and 0xFF).toByte()
    }

    /**
     * 将 Int 编码为 4 字节小端
     */
    private fun uIntLe(v: Int): ByteArray = ByteArray(4) { i ->
        ((v ushr (i * 8)) and 0xFF).toByte()
    }

    /**
     * 从偏移处读取 8 字节小端 Long
     */
    private fun readULongLe(data: ByteArray, offset: Int): Long {
        var v = 0L
        for (i in 0 until 8) {
            v = v or ((data[offset + i].toLong() and 0xFF) shl (i * 8))
        }
        return v
    }

    /**
     * 从偏移处读取 4 字节小端 Int
     */
    private fun readUIntLe(data: ByteArray, offset: Int): Int {
        var v = 0
        for (i in 0 until 4) {
            v = v or ((data[offset + i].toInt() and 0xFF) shl (i * 8))
        }
        return v
    }

    /**
     * 读取 u64 长度前缀字符串，返回 (字符串, 下一个读取位置)
     */
    private fun readLengthPrefixedU64(data: ByteArray, offset: Int): Pair<String, Int> {
        val len = readULongLe(data, offset).toInt()
        val start = offset + 8
        val str = data.decodeToString(start, start + len)
        return str to (start + len)
    }

    /**
     * 读取 u32 长度前缀字符串，返回 (字符串, 下一个读取位置)
     */
    private fun readLengthPrefixedU32(data: ByteArray, offset: Int): Pair<String, Int> {
        val len = readUIntLe(data, offset)
        val start = offset + 4
        val str = data.decodeToString(start, start + len)
        return str to (start + len)
    }
}
