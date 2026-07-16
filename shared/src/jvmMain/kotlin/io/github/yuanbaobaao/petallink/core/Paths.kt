package io.github.yuanbaobaao.petallink.core

import java.nio.file.Path
import java.nio.file.Paths

/**
 * 安全路径函数（对标 src/core/paths.rs + cache_paths.rs）。
 *
 * 详见 docs/04 §8。
 */

/** 展开 ~ 为 home 目录 */
fun expandTilde(path: String): String {
    if (path.startsWith("~/")) {
        return System.getProperty("user.home") + path.substring(1)
    }
    return path
}

/** 校验路径段合法（拒绝空/./../斜杠/反斜杠/空字符） */
fun validatePathSegment(segment: String) {
    require(segment.isNotEmpty()) { "路径段为空" }
    require(segment != "." && segment != "..") { "路径段为目录引用: $segment" }
    require('/' !in segment) { "路径段含斜杠: $segment" }
    require('\\' !in segment) { "路径段含反斜杠: $segment" }
    require('\u0000' !in segment) { "路径段含空字符" }
}

/** 校验相对路径（拒绝绝对路径/上跳/反斜杠/空字符） */
fun validateRelativePath(relPath: String, allowEmpty: Boolean = false) {
    require(allowEmpty || relPath.isNotEmpty()) { "相对路径为空" }
    require('\\' !in relPath) { "相对路径含反斜杠: $relPath" }
    require('\u0000' !in relPath) { "相对路径含空字符" }
    require(!Path.of(relPath).isAbsolute) { "相对路径为绝对路径: $relPath" }
    // 逐个组件校验
    for (component in Path.of(relPath)) {
        val seg = component.toString()
        if (seg == "..") throw IllegalArgumentException("相对路径含 .. 上跳: $relPath")
        if (seg != ".") validatePathSegment(seg)
    }
}

/** 安全拼接：base + 已验证的相对路径 */
fun safeJoinUnder(base: Path, relPath: String, allowEmpty: Boolean = false): Path {
    validateRelativePath(relPath, allowEmpty)
    return base.resolve(relPath)
}

/** 缓存文件路径（escape 规则：非 [A-Za-z0-9._-] 替换为 _） */
fun escapeMountPath(absPath: String): String {
    return absPath.map { if (it in 'A'..'Z' || it in 'a'..'z' || it in '0'..'9' || it in "._-") it else '_' }.joinToString("")
}

fun cacheBaseDir(): Path {
    val home = System.getProperty("user.home")
    return Paths.get(home, "Library", "Application Support", "PetalLink")
}

fun cloudTreeCacheFile(absMountDir: String): Path = cacheBaseDir().resolve("cloudtree_${escapeMountPath(absMountDir)}.json")
fun changesCursorFile(absMountDir: String): Path = cacheBaseDir().resolve("changes_cursor_${escapeMountPath(absMountDir)}.txt")

/** 清除缓存文件 */
fun clearCacheFiles(absMountDir: String) {
    java.nio.file.Files.deleteIfExists(cloudTreeCacheFile(absMountDir))
    java.nio.file.Files.deleteIfExists(changesCursorFile(absMountDir))
}
