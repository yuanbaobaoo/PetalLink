package io.github.yuanbaobaoo.petallink.mount

import com.sun.jna.Library
import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Platform
import com.sun.jna.Pointer
import io.github.yuanbaobaoo.petallink.AppError

/**
 * 通过 macOS libc 直接调用 getxattr/setxattr/removexattr。
 */
object MacXattrAccess : XattrAccess {
    private const val ENOATTR_MAC = 93
    private const val ENODATA_LINUX = 61
    private const val ERANGE = 34

    /**
     * macOS libc 的 JNA 绑定接口，提供 getxattr/setxattr/removexattr 扩展属性操作。
     */
    private interface LibC : Library {
        /**
         * macOS libc getxattr 的 JNA 声明，读取指定路径的扩展属性值，返回值长度或 -1。
         */
        fun getxattr(path: String, name: String, value: Pointer?, size: Long, position: Int, options: Int): Long
        /**
         * macOS libc setxattr 的 JNA 声明，写入指定路径的扩展属性值，返回 0 表示成功。
         */
        fun setxattr(path: String, name: String, value: Pointer?, size: Long, position: Int, options: Int): Int
        /**
         * macOS libc removexattr 的 JNA 声明，删除指定路径的扩展属性，返回 0 表示成功。
         */
        fun removexattr(path: String, name: String, options: Int): Int
    }

    private val libc: LibC by lazy {
        if (!Platform.isMac()) throw AppError.LocalIo("xattr 实现仅支持 macOS")
        Native.load(Platform.C_LIBRARY_NAME, LibC::class.java)
    }

    /**
     * 读取扩展属性；属性不存在返回 null，并容忍读取期间属性大小变化重试若干次。
     */
    override fun get(path: String, name: String): ByteArray? {
        repeat(3) {
            val size = libc.getxattr(path, name, null, 0, 0, 0)
            if (size < 0) {
                val errno = Native.getLastError()
                if (errno == ENOATTR_MAC || errno == ENODATA_LINUX) return null
                fail("getxattr(size)", path, name, errno)
            }
            if (size == 0L) return ByteArray(0)
            val memory = Memory(size)
            val read = libc.getxattr(path, name, memory, size, 0, 0)
            if (read >= 0) return memory.getByteArray(0, read.toInt())
            val errno = Native.getLastError()
            if (errno != ERANGE) {
                if (errno == ENOATTR_MAC || errno == ENODATA_LINUX) return null
                fail("getxattr(value)", path, name, errno)
            }
        }
        throw AppError.LocalIo("getxattr 连续因属性大小变化失败: $path [$name]")
    }

    /**
     * 写入扩展属性值，失败时抛出携带 errno 的异常。
     */
    override fun set(path: String, name: String, value: ByteArray) {
        val memory = if (value.isEmpty()) null else Memory(value.size.toLong()).also {
            it.write(0, value, 0, value.size)
        }
        if (libc.setxattr(path, name, memory, value.size.toLong(), 0, 0) != 0) {
            fail("setxattr", path, name, Native.getLastError())
        }
    }

    /**
     * 删除扩展属性，属性已不存在时静默返回。
     */
    override fun remove(path: String, name: String) {
        if (libc.removexattr(path, name, 0) != 0) {
            val errno = Native.getLastError()
            if (errno == ENOATTR_MAC || errno == ENODATA_LINUX) return
            fail("removexattr", path, name, errno)
        }
    }

    /**
     * 将 xattr 操作的 errno 包装为 [AppError.LocalIo] 并抛出，永不正常返回。
     */
    private fun fail(operation: String, path: String, name: String, errno: Int): Nothing =
        throw AppError.LocalIo("$operation 失败 errno=$errno: $path [$name]")
}
