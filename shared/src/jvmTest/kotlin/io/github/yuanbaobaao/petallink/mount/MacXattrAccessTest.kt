package io.github.yuanbaobaao.petallink.mount

import java.nio.file.Files
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals

class MacXattrAccessTest {
    @Test
    fun 真实libcXattr可读写删除() {
        if (!System.getProperty("os.name").contains("Mac", ignoreCase = true)) return
        val file = Files.createTempFile("petallink-xattr-", ".tmp")
        val name = "com.hwcloud.test"
        assertEquals(null, MacXattrAccess.get(file.toString(), name))
        MacXattrAccess.set(file.toString(), name, byteArrayOf(0, 1, 2, 0xff.toByte()))
        assertContentEquals(byteArrayOf(0, 1, 2, 0xff.toByte()), MacXattrAccess.get(file.toString(), name))
        MacXattrAccess.remove(file.toString(), name)
        assertEquals(null, MacXattrAccess.get(file.toString(), name))
        MacXattrAccess.remove(file.toString(), name)
    }
}
