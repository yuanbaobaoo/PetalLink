package io.github.yuanbaobaao.petallink

/**
 * 平台信息（expect，macosMain 提供 actual）。
 */

/** 当前运行平台名称（用于日志/诊断） */
expect fun platformName(): String

// ------------------------------------------------------------------
// expect 平台能力声明（macosMain 提供 actual）
// 对标原项目 src/ 中各平台相关模块。
// ------------------------------------------------------------------

/** inode 读取（docs/11 §4.2）：读取文件系统 inode 编号 */
expect object PlatformInode {
    /** 读取指定绝对路径的 inode，失败抛 [AppError.LocalIo] */
    fun readInode(absolutePath: String): ULong
}
