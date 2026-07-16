package io.github.yuanbaobaao.petallink.sync.engine

/**
 * 平台文件操作（expect，macosMain 提供 actual）。
 * 用于 executeDownload 的 POSIX rename + delete。
 */

/** POSIX rename（同文件系统原子操作） */
expect fun platformRenameExpect(from: String, to: String)

/** 删除文件（清理 .tmp 残留） */
expect fun platformDeleteExpect(path: String)
