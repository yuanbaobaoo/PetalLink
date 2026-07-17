package io.github.yuanbaobaoo.petallink.mount

/**
 * 本地文件扫描条目（对标原项目 LocalFileEntry）。
 *
 * 含 inode 字段（docs/11 §4.2 inode 身份识别）。
 */
data class LocalFileEntry(
    val absolutePath: String,   // 经过 normalize 的绝对路径
    val relativePath: String,   // 相对挂载目录的路径
    val inode: ULong,           // 文件系统 inode（meta.ino()）
    val size: Long,             // 字节数
    val mtime: Long,            // 修改时间（epoch ms）
    val isDirectory: Boolean,
    val isPlaceholder: Boolean, // 占位符判定：size==0 AND xattr state=="placeholder"
    val placeholderState: PlaceholderState? = null,
)

/**
 * 同步扫描器抽象；实现必须递归且不跟随符号链接。
 */
interface LocalFileScanner {
    /**
     * 递归扫描挂载目录（不跟随符号链接）返回全部本地条目
     */
    suspend fun scan(): List<LocalFileEntry>
}

/**
 * xattr 最小平台抽象；缺失属性返回 null，其他失败抛 LocalIo。
 */
interface XattrAccess {
    /**
     * 读取指定名称的扩展属性，缺失返回 null，其他失败抛异常
     */
    fun get(path: String, name: String): ByteArray?

    /**
     * 设置指定名称的扩展属性值
     */
    fun set(path: String, name: String, value: ByteArray)

    /**
     * 删除指定名称的扩展属性
     */
    fun remove(path: String, name: String)
}

/**
 * 占位符状态 xattr 值（对标 com.hwcloud.state）。
 * 详见 docs/11 §2.1（inode 方案后仅 2 个 xattr 键）。
 */
enum class PlaceholderState(val xattrValue: String) {
    /**
     * 占位符（0 字节，云端文件未下载）
     */
    PLACEHOLDER("placeholder"),

    /**
     * 已下载（真实内容）
     */
    DOWNLOADED("downloaded");

    companion object {
        /**
         * 从 xattr 字符串值解析占位符状态，无法识别返回 null
         */
        fun fromXattr(value: String?): PlaceholderState? = when (value) {
            "placeholder" -> PLACEHOLDER
            "downloaded" -> DOWNLOADED
            else -> null
        }
    }
}

/**
 * 占位符管理接口（对标 src/mount/manager.rs 占位符相关函数）。
 * 实现由 macosMain 提供（xattr 读写 + 文件操作）。
 *
 * 详见 docs/04 §9、docs/11 §4.9。
 * inode 方案后 create_placeholder 只写 1 个 xattr（state）。
 */
interface PlaceholderManager {
    /**
     * 创建占位符（如果文件已存在且无 state xattr → 视为用户文件，绝不转换）。
     * @return true 表示创建了占位符；false 表示文件已存在且是用户文件
     */
    suspend fun createPlaceholderIfNeeded(relativePath: String): Boolean

    /**
     * 严格版占位符创建（破坏性流程专用，不做检查直接 create_new）。
     */
    suspend fun createPlaceholderStrict(relativePath: String)

    /**
     * 判定是否为占位符（size==0 AND xattr state=="placeholder"）。
     * .gitkeep 等用户 0 字节文件受保护（无 state xattr → false）。
     */
    suspend fun isPlaceholder(absolutePath: String): Boolean

    /**
     * 真实内容完整落盘后标记 downloaded 并清除灰标。
     */
    suspend fun markDownloaded(absolutePath: String)

    /**
     * 设置/清除 Finder 灰标（buf[9]=0x02，纯视觉）。
     */
    suspend fun setFinderGreyLabel(absolutePath: String, on: Boolean)

    /**
     * 删除本地文件（0 字节必须是 placeholder 才删，保护 .gitkeep）。
     */
    suspend fun deleteLocal(absolutePath: String)

    /**
     * 备份被修改的占位符（state=placeholder 但 size>0 → 改名 .local-timestamp 备份）。
     */
    suspend fun backupModifiedPlaceholder(absolutePath: String): String?
}
