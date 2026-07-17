package io.github.yuanbaobaoo.petallink.platform

import java.math.BigInteger

/**
 * Poly1305 MAC 纯 Kotlin 实现（RFC 8439 §2.5）。
 *
 * 用于 ChaCha20-Poly1305 AEAD 的认证标签。
 * key: 32 字节（r || s），r 被 clamp。
 */
object Poly1305 {

    private val P: BigInteger = BigInteger("3fffffffffffffffffffffffffffffffb", 16)  // 2^130 - 5

    /**
     * 计算 Poly1305 MAC。
     * @param key 32 字节密钥（前 16 字节 r，后 16 字节 s）
     * @param data 待认证数据
     * @return 16 字节 tag
     */
    fun mac(key: ByteArray, data: ByteArray): ByteArray {
        require(key.size == 32) { "Poly1305 key 必须是 32 字节" }

        // r = key[0..16]，clamp
        val rBytes = key.copyOfRange(0, 16)
        clampR(rBytes)
        val r = bytesToBigInteger(rBytes)
        val s = bytesToBigInteger(key.copyOfRange(16, 32))

        // 累加器
        var acc = BigInteger.ZERO

        // 分块处理（每块 16 字节，追加 0x01）
        var offset = 0
        while (offset < data.size) {
            val blockLen = minOf(16, data.size - offset)
            val block = ByteArray(17)  // 16 字节 + 1 字节 0x01
            for (i in 0 until blockLen) {
                block[i] = data[offset + i]
            }
            block[blockLen] = 1  // 追加 0x01

            val n = leBytesToBigInteger(block, blockLen + 1)
            acc = (acc + n) * r % P
            offset += 16
        }

        // tag = (acc + s) mod 2^128
        val tagInt = (acc + s).mod(BigInteger.valueOf(1).shiftLeft(128))
        return bigIntegerTo16BytesLE(tagInt)
    }

    /** Clamp r：清除 r[3]、r[7]、r[11]、r[15] 的高 4 位 */
    private fun clampR(r: ByteArray) {
        r[3] = (r[3].toInt() and 0x0F).toByte()
        r[7] = (r[7].toInt() and 0x0F).toByte()
        r[11] = (r[11].toInt() and 0x0F).toByte()
        r[15] = (r[15].toInt() and 0x0F).toByte()
        r[4] = (r[4].toInt() and 0xFC).toByte()
        r[8] = (r[8].toInt() and 0xFC).toByte()
        r[12] = (r[12].toInt() and 0xFC).toByte()
    }

    /** 小端字节数组 → BigInteger（无符号） */
    private fun leBytesToBigInteger(data: ByteArray, len: Int): BigInteger {
        val reversed = ByteArray(len)
        for (i in 0 until len) {
            reversed[i] = data[len - 1 - i]
        }
        // 确保正数
        val positive = ByteArray(len + 1)
        System.arraycopy(reversed, 0, positive, 1, len)
        return BigInteger(positive)
    }

    /** 字节数组 → BigInteger（无符号，大端） */
    private fun bytesToBigInteger(data: ByteArray): BigInteger {
        val positive = ByteArray(data.size + 1)
        System.arraycopy(data.reversedArray(), 0, positive, 1, data.size)
        return BigInteger(positive)
    }

    /** BigInteger → 16 字节小端 */
    private fun bigIntegerTo16BytesLE(value: BigInteger): ByteArray {
        val bytes = value.toByteArray()
        val result = ByteArray(16)
        // 去除前导零字节
        val start = if (bytes[0] == 0.toByte()) 1 else 0
        val len = bytes.size - start
        for (i in 0 until minOf(len, 16)) {
            result[i] = bytes[start + len - 1 - i]  // 小端
        }
        return result
    }
}
