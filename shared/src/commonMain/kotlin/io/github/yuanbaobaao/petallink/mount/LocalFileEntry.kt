package io.github.yuanbaobaao.petallink.mount

/**
 * 本地文件扫描条目（对标原项目 LocalFileEntry）。
 *
 * 含 inode 字段（docs/11 §4.2 inode 身份识别）。
 */
data class LocalFileEntry(
    val relativePath: String,   // 相对挂载目录的路径
    val inode: ULong,           // 文件系统 inode（meta.ino()）
    val size: Long,             // 字节数
    val mtime: Long,            // 修改时间（秒）
    val isDirectory: Boolean,
    val isPlaceholder: Boolean, // 占位符判定：size==0 AND xattr state=="placeholder"
)

/**
 * 占位符状态 xattr 值（对标 com.hwcloud.state）。
 * 详见 docs/11 §2.1（inode 方案后仅 2 个 xattr 键）。
 */
enum class PlaceholderState(val xattrValue: String) {
    /** 占位符（0 字节，云端文件未下载） */
    PLACEHOLDER("placeholder"),
    /** 已下载（真实内容） */
    DOWNLOADED("downloaded");

    companion object {
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
