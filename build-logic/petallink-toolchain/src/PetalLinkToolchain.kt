package io.github.yuanbaobaoo.petallink.toolchain

import org.jetbrains.amper.plugins.Input
import org.jetbrains.amper.plugins.Output
import org.jetbrains.amper.plugins.TaskAction
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.util.Properties
import kotlin.io.path.createDirectories
import kotlin.io.path.isDirectory
import kotlin.io.path.isExecutable
import kotlin.io.path.writeText

private const val PROD_BUNDLE_ID = "io.github.yuanbaobaoo.PetalLink"
private const val DEFAULT_UPDATE_ENDPOINT =
    "https://github.com/yuanbaobaoo/PetalLink/releases/latest/download/PetalLink-update.json"

/**
 * 为 Kotlin Toolchain 构建生成与 Gradle 版本等价的 BuildInfo。
 *
 * Toolchain 的日常 build/test/run 默认始终使用 dev 档案；如需检查 release 编译信息，可显式设置
 * PETALLINK_BUILD_PROFILE=release。正式 DMG 仍由 releaseDmg 命令委托既有发布链生成。
 */
@TaskAction
fun generateBuildInfo(
    @Input versionPropertiesFile: Path,
    @Output generatedSourceDir: Path,
) {
    val properties = Properties().apply {
        versionPropertiesFile.toFile().bufferedReader().use { reader -> load(reader) }
    }
    val version = properties.getProperty("petalLinkVersion")?.trim().orEmpty()
    require(version.isNotEmpty()) {
        "Missing petalLinkVersion in $versionPropertiesFile"
    }

    val environment = System.getenv()
    val buildProfile = environment["PETALLINK_BUILD_PROFILE"]
        ?.trim()
        ?.lowercase()
        ?.takeIf { it == "dev" || it == "release" }
        ?: "dev"
    val bundleId = if (buildProfile == "release") PROD_BUNDLE_ID else "$PROD_BUNDLE_ID-dev"
    val updateEndpoint = environment["PETALLINK_UPDATE_ENDPOINT"]
        ?.takeIf(String::isNotBlank)
        ?: DEFAULT_UPDATE_ENDPOINT
    val updateTeamId = environment["PETALLINK_UPDATE_TEAM_ID"].orEmpty()

    generatedSourceDir.createDirectories()
    generatedSourceDir.resolve("BuildInfo.kt").writeText(
        """
        package io.github.yuanbaobaoo.petallink.core

        object BuildInfo {
            const val VERSION: String = ${version.asKotlinLiteral()}
            const val BUILD_PROFILE: String = ${buildProfile.asKotlinLiteral()}
            const val BUNDLE_ID: String = ${bundleId.asKotlinLiteral()}
            const val UPDATE_ENDPOINT: String = ${updateEndpoint.asKotlinLiteral()}
            const val UPDATE_TEAM_ID: String = ${updateTeamId.asKotlinLiteral()}
        }
        """.trimIndent() + "\n",
    )
    println("Generated BuildInfo $version ($buildProfile)")
}

/** 通过 Kotlin Toolchain 自定义命令调用暂未被原生支持的 Compose Desktop 分发任务。 */
@TaskAction
fun runGradle(
    @Input unixWrapper: Path,
    @Input windowsWrapper: Path,
    arguments: List<String>,
) {
    val isWindows = System.getProperty("os.name").startsWith("Windows", ignoreCase = true)
    val wrapper = if (isWindows) windowsWrapper else unixWrapper
    require(wrapper.toFile().isFile) { "Missing internal build wrapper: $wrapper" }

    val command = if (isWindows) {
        listOf("cmd.exe", "/d", "/c", wrapper.toString()) + arguments
    } else {
        listOf(wrapper.toString()) + arguments
    }
    val process = ProcessBuilder(command)
        .directory(unixWrapper.parent.toFile())
        .inheritIO()
    val packagingJdk = findPackagingJdk()
    process.environment()["JAVA_HOME"] = packagingJdk.toString()
    process.environment()["PATH"] = listOf(
        packagingJdk.resolve("bin"),
        process.environment()["PATH"].orEmpty(),
    ).joinToString(System.getProperty("path.separator"))
    println("Using Kotlin Toolchain managed JDK for desktop packaging: $packagingJdk")

    val exitCode = process.start().waitFor()
    check(exitCode == 0) {
        "Internal desktop packaging failed with exit code $exitCode"
    }
}

private fun findPackagingJdk(): Path {
    val override = System.getenv("PETALLINK_PACKAGING_JAVA_HOME")
        ?.takeIf(String::isNotBlank)
        ?.let(Paths::get)
    if (override != null) {
        require(override.hasPackagingTools()) {
            "PETALLINK_PACKAGING_JAVA_HOME does not contain jlink and jpackage: $override"
        }
        return override
    }

    val cacheRoot = toolchainCacheRoot()
    val extractCache = cacheRoot.resolve("extract.cache")
    require(extractCache.isDirectory()) {
        "Kotlin Toolchain JDK cache does not exist: $extractCache"
    }

    val candidates = Files.find(
        extractCache,
        8,
        { path, _ -> path.fileName.toString() == "jpackage" && path.isExecutable() },
    ).use { paths ->
        paths
            .map { it.parent.parent }
            .filter { it.hasPackagingTools() && it.isJdk25() }
            .toList()
    }
    return candidates.maxByOrNull { Files.getLastModifiedTime(it.resolve("release")).toMillis() }
        ?: error("Kotlin Toolchain did not provision a full JDK 25 under $extractCache")
}

private fun toolchainCacheRoot(): Path {
    System.getenv("KOTLIN_SHARED_CACHE_DIR")
        ?.takeIf(String::isNotBlank)
        ?.let(Paths::get)
        ?.let { return it }
    System.getenv("AMPER_SHARED_CACHE_DIR")
        ?.takeIf(String::isNotBlank)
        ?.let(Paths::get)
        ?.let { return it }

    val osName = System.getProperty("os.name").lowercase()
    val userHome = Paths.get(System.getProperty("user.home"))
    return when {
        osName.contains("mac") -> userHome.resolve("Library/Caches/JetBrains/Kotlin")
        osName.contains("win") -> Paths.get(
            requireNotNull(System.getenv("LOCALAPPDATA")) { "LOCALAPPDATA is not set" },
            "JetBrains",
            "Kotlin",
        )
        else -> System.getenv("XDG_CACHE_HOME")
            ?.takeIf(String::isNotBlank)
            ?.let(Paths::get)
            ?.resolve("JetBrains/Kotlin")
            ?: userHome.resolve(".cache/JetBrains/Kotlin")
    }
}

private fun Path.hasPackagingTools(): Boolean =
    resolve("bin/jlink").isExecutable() && resolve("bin/jpackage").isExecutable()

private fun Path.isJdk25(): Boolean {
    val releaseFile = resolve("release")
    if (!Files.isRegularFile(releaseFile)) return false
    val javaVersion = Files.readAllLines(releaseFile)
        .firstOrNull { it.startsWith("JAVA_VERSION=") }
        ?.substringAfter('=')
        ?.trim('"')
        ?: return false
    return javaVersion.substringBefore('.').toIntOrNull() == 25
}

private fun String.asKotlinLiteral(): String = buildString {
    append('"')
    for (character in this@asKotlinLiteral) {
        when (character) {
            '\\' -> append("\\\\")
            '"' -> append("\\\"")
            '\n' -> append("\\n")
            '\r' -> append("\\r")
            '\t' -> append("\\t")
            '$' -> append("\\$")
            else -> append(character)
        }
    }
    append('"')
}
