package io.github.yuanbaobaoo.petallink.sync.engine

/**
 * 纯 Kotlin SHA-256 实现（无外部依赖，KMP commonMain 可用）。
 *
 * 用于下载文件 sha256 校验（对标原项目 sha256_file，1MB buffer 流式）。
 * 详见 docs/03 §下载。
 */
private object Sha256 {
    private val K = intArrayOf(
        0x428a2f98, 0x71374491, 0xb5c0fbcf.toInt(), 0xe9b5dba5.toInt(),
        0x3956c25b, 0x59f111f1, 0x923f82a4.toInt(), 0xab1c5ed5.toInt(),
        0xd807aa98.toInt(), 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe.toInt(), 0x9bdc06a7.toInt(), 0xc19bf174.toInt(),
        0xe49b69c1.toInt(), 0xefbe4786.toInt(), 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152.toInt(), 0xa831c66d.toInt(), 0xb00327c8.toInt(), 0xbf597fc7.toInt(),
        0xc6e00bf3.toInt(), 0xd5a79147.toInt(), 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e.toInt(), 0x92722c85.toInt(),
        0xa2bfe8a1.toInt(), 0xa81a664b.toInt(), 0xc24b8b70.toInt(), 0xc76c51a3.toInt(),
        0xd192e819.toInt(), 0xd6990624.toInt(), 0xf40e3585.toInt(), 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
        0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814.toInt(), 0x8cc70208.toInt(),
        0x90befffa.toInt(), 0xa4506ceb.toInt(), 0xbef9a3f7.toInt(), 0xc67178f2.toInt(),
    )

    /**
     * 32 位循环右移
     */
    private fun rotr(x: Int, n: Int): Int = (x ushr n) or (x shl (32 - n))

    /**
     * 计算输入数据的 SHA-256 摘要（32 字节）
     */
    fun hash(data: ByteArray): ByteArray {
        var h0 = 0x6a09e667; var h1 = 0xbb67ae85.toInt()
        var h2 = 0x3c6ef372; var h3 = 0xa54ff53a.toInt()
        var h4 = 0x510e527f; var h5 = 0x9b05688c.toInt()
        var h6 = 0x1f83d9ab; var h7 = 0x5be0cd19

        // 填充：补 0x80 + 0... + 64 位长度
        val bitLen = data.size.toLong() * 8
        val padded = ByteArray(((data.size + 9 + 63) / 64) * 64)
        data.copyInto(padded)
        padded[data.size] = 0x80.toByte()
        // 末尾 8 字节为大端位长度
        for (i in 0 until 8) {
            padded[padded.size - 1 - i] = ((bitLen ushr (i * 8)) and 0xFF).toByte()
        }

        val w = IntArray(64)
        var offset = 0
        while (offset < padded.size) {
            // 前 16 个字（大端）
            for (i in 0 until 16) {
                w[i] = ((padded[offset + i * 4].toInt() and 0xFF) shl 24) or
                    ((padded[offset + i * 4 + 1].toInt() and 0xFF) shl 16) or
                    ((padded[offset + i * 4 + 2].toInt() and 0xFF) shl 8) or
                    (padded[offset + i * 4 + 3].toInt() and 0xFF)
            }
            // 扩展到 64 个字
            for (i in 16 until 64) {
                val s0 = rotr(w[i - 15], 7) xor rotr(w[i - 15], 18) xor (w[i - 15] ushr 3)
                val s1 = rotr(w[i - 2], 17) xor rotr(w[i - 2], 19) xor (w[i - 2] ushr 10)
                w[i] = w[i - 16] + s0 + w[i - 7] + s1
            }

            var a = h0; var b = h1; var c = h2; var d = h3
            var e = h4; var f = h5; var g = h6; var h = h7

            for (i in 0 until 64) {
                val s1 = rotr(e, 6) xor rotr(e, 11) xor rotr(e, 25)
                val ch = (e and f) xor (e.inv() and g)
                val temp1 = h + s1 + ch + K[i] + w[i]
                val s0 = rotr(a, 2) xor rotr(a, 13) xor rotr(a, 22)
                val maj = (a and b) xor (a and c) xor (b and c)
                val temp2 = s0 + maj

                h = g; g = f; f = e
                e = d + temp1
                d = c; c = b; b = a
                a = temp1 + temp2
            }

            h0 += a; h1 += b; h2 += c; h3 += d
            h4 += e; h5 += f; h6 += g; h7 += h

            offset += 64
        }

        val result = ByteArray(32)
        val hashes = intArrayOf(h0, h1, h2, h3, h4, h5, h6, h7)
        for (i in hashes.indices) {
            result[i * 4] = ((hashes[i] ushr 24) and 0xFF).toByte()
            result[i * 4 + 1] = ((hashes[i] ushr 16) and 0xFF).toByte()
            result[i * 4 + 2] = ((hashes[i] ushr 8) and 0xFF).toByte()
            result[i * 4 + 3] = (hashes[i] and 0xFF).toByte()
        }
        return result
    }
}

/**
 * 计算 SHA-256 并返回小写十六进制字符串
 */
fun sha256Pure(data: ByteArray): String {
    val bytes = Sha256.hash(data)
    val sb = StringBuilder(bytes.size * 2)
    val HEX = "0123456789abcdef"
    for (b in bytes) {
        val v = b.toInt() and 0xFF
        sb.append(HEX[v ushr 4])
        sb.append(HEX[v and 0xF])
    }
    return sb.toString()
}
