package io.github.yuanbaobaao.petallink.platform

/**
 * ChaCha20-Poly1305 AEAD 纯 Kotlin 实现（RFC 8439）。
 *
 * 用于 token.bin 加密存储（对标原项目 ChaCha20-Poly1305 AEAD）。
 * 详见 docs/07 §token.bin。
 *
 * - key: 32 字节
 * - nonce: 12 字节
 * - 输出：ciphertext（与明文等长）+ 16 字节 Poly1305 tag
 */
object ChaCha20Poly1305 {

    /**
     * AEAD 加密。
     * @param key 32 字节密钥
     * @param nonce 12 字节 nonce
     * @param plaintext 明文
     * @return ciphertext + tag（明文长度 + 16）
     */
    fun encrypt(key: ByteArray, nonce: ByteArray, plaintext: ByteArray): ByteArray {
        require(key.size == 32) { "key 必须是 32 字节" }
        require(nonce.size == 12) { "nonce 必须是 12 字节" }

        // 1. ChaCha20 流加密（block counter 从 1 开始，block 0 用于 Poly1305 key）
        val ciphertext = chaCha20Encrypt(key, 0, nonce, plaintext, counter = 1)

        // 2. 生成 Poly1305 one-time key（block 0）
        val polyKey = chaCha20Block(key, 0, nonce).copyOfRange(0, 32)
        // 取前 32 字节，每个低 4 字节 cleared（clamp），但 Poly1305 内部处理

        // 3. 计算 Poly1305 MAC over: padded AAD (空) || padded ciphertext || len(AAD) || len(ct)
        val macData = buildMacData(ciphertext)
        val tag = Poly1305.mac(polyKey, macData)

        return ciphertext + tag
    }

    /**
     * AEAD 解密。
     * @param key 32 字节密钥
     * @param nonce 12 字节 nonce
     * @param ciphertextWithTag ciphertext + 16 字节 tag
     * @return 明文；tag 校验失败抛异常
     */
    fun decrypt(key: ByteArray, nonce: ByteArray, ciphertextWithTag: ByteArray): ByteArray {
        require(key.size == 32) { "key 必须是 32 字节" }
        require(nonce.size == 12) { "nonce 必须是 12 字节" }
        require(ciphertextWithTag.size >= 16) { "密文太短（含 tag 至少 16 字节）" }

        val ctLen = ciphertextWithTag.size - 16
        val ciphertext = ciphertextWithTag.copyOfRange(0, ctLen)
        val tag = ciphertextWithTag.copyOfRange(ctLen, ciphertextWithTag.size)

        // 1. 验证 Poly1305 tag
        val polyKey = chaCha20Block(key, 0, nonce).copyOfRange(0, 32)
        val macData = buildMacData(ciphertext)
        val expectedTag = Poly1305.mac(polyKey, macData)

        if (!constantTimeEquals(tag, expectedTag)) {
            throw SecurityException("Poly1305 tag 校验失败")
        }

        // 2. ChaCha20 解密（counter 从 1 开始）
        return chaCha20Encrypt(key, 0, nonce, ciphertext, counter = 1)
    }

    /**
     * 构建 Poly1305 MAC 输入数据。
     * RFC 8439: pad16(AAD) || pad16(ciphertext) || len(AAD) as u64le || len(ct) as u64le
     * AAD 为空，所以只有 pad16(ct) || 0u64 || len(ct)u64
     */
    private fun buildMacData(ciphertext: ByteArray): ByteArray {
        val paddedCt = padTo16(ciphertext)
        val lenBlock = ByteArray(16)
        // len(AAD)=0 (8 bytes LE) + len(ct) (8 bytes LE)
        writeU64LE(lenBlock, 8, ciphertext.size.toLong())
        return paddedCt + lenBlock
    }

    /** 填充到 16 字节边界 */
    private fun padTo16(data: ByteArray): ByteArray {
        val rem = data.size % 16
        if (rem == 0) return data
        return data + ByteArray(16 - rem)
    }

    /** 小端写入 u64 */
    private fun writeU64LE(buf: ByteArray, offset: Int, value: Long) {
        for (i in 0 until 8) {
            buf[offset + i] = ((value ushr (i * 8)) and 0xFF).toByte()
        }
    }

    /** 常数时间比较（防时序攻击） */
    private fun constantTimeEquals(a: ByteArray, b: ByteArray): Boolean {
        if (a.size != b.size) return false
        var result = 0
        for (i in a.indices) {
            result = result or (a[i].toInt() xor b[i].toInt())
        }
        return result == 0
    }

    // --- ChaCha20 流密码 ---

    /**
     * ChaCha20 加密/解密（XOR 流）。
     * @param key 32 字节
     * @param counter 初始计数器（通常传 0，block counter 由内部递增）
     * @param nonce 12 字节
     * @param data 输入数据
     * @param counter 起始 block counter（AEAD 中加密用 1，Poly1305 key 用 0）
     */
    private fun chaCha20Encrypt(
        key: ByteArray, @Suppress("UNUSED_PARAMETER") unused: Int,
        nonce: ByteArray, data: ByteArray, counter: Int,
    ): ByteArray {
        val output = ByteArray(data.size)
        var blockCounter = counter
        var offset = 0

        while (offset < data.size) {
            val block = chaCha20Block(key, blockCounter, nonce)
            val blockLen = minOf(64, data.size - offset)
            for (i in 0 until blockLen) {
                output[offset + i] = (data[offset + i].toInt() xor block[i].toInt()).toByte()
            }
            offset += blockLen
            blockCounter++
        }
        return output
    }

    /**
     * ChaCha20 单块（64 字节 keystream，RFC 8439 §2.3）。
     */
    fun chaCha20Block(key: ByteArray, counter: Int, nonce: ByteArray): ByteArray {
        val state = IntArray(16)

        // 常量 "expand 32-byte k"
        state[0] = 0x61707865
        state[1] = 0x3320646e
        state[2] = 0x79622d32
        state[3] = 0x6b206574

        // Key (8 words, little-endian)
        for (i in 0 until 8) {
            state[4 + i] = leU32(key, i * 4)
        }

        // Counter
        state[12] = counter

        // Nonce (3 words, little-endian)
        state[13] = leU32(nonce, 0)
        state[14] = leU32(nonce, 4)
        state[15] = leU32(nonce, 8)

        // 20 轮（10 次 double round）
        val working = state.copyOf()
        for (i in 0 until 10) {
            quarterRound(working, 0, 4, 8, 12)
            quarterRound(working, 1, 5, 9, 13)
            quarterRound(working, 2, 6, 10, 14)
            quarterRound(working, 3, 7, 11, 15)
            quarterRound(working, 0, 5, 10, 15)
            quarterRound(working, 1, 6, 11, 12)
            quarterRound(working, 2, 7, 8, 13)
            quarterRound(working, 3, 4, 9, 14)
        }

        // 加初始状态，输出小端字节
        val output = ByteArray(64)
        for (i in 0 until 16) {
            val v = working[i] + state[i]
            writeU32LE(output, i * 4, v)
        }
        return output
    }

    /** ChaCha20 quarter round */
    private fun quarterRound(s: IntArray, a: Int, b: Int, c: Int, d: Int) {
        s[a] += s[b]; s[d] = rotl32(s[d] xor s[a], 16)
        s[c] += s[d]; s[b] = rotl32(s[b] xor s[c], 12)
        s[a] += s[b]; s[d] = rotl32(s[d] xor s[a], 8)
        s[c] += s[d]; s[b] = rotl32(s[b] xor s[c], 7)
    }

    private fun rotl32(x: Int, n: Int): Int = (x shl n) or (x ushr (32 - n))

    /** 小端读取 4 字节为 UInt */
    private fun leU32(buf: ByteArray, offset: Int): Int =
        (buf[offset].toInt() and 0xFF) or
        ((buf[offset + 1].toInt() and 0xFF) shl 8) or
        ((buf[offset + 2].toInt() and 0xFF) shl 16) or
        ((buf[offset + 3].toInt() and 0xFF) shl 24)

    /** 小端写入 4 字节 */
    private fun writeU32LE(buf: ByteArray, offset: Int, value: Int) {
        buf[offset] = (value and 0xFF).toByte()
        buf[offset + 1] = ((value ushr 8) and 0xFF).toByte()
        buf[offset + 2] = ((value ushr 16) and 0xFF).toByte()
        buf[offset + 3] = ((value ushr 24) and 0xFF).toByte()
    }
}
