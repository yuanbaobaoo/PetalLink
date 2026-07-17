package io.github.yuanbaobaoo.petallink.update

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.core.AppPaths
import io.ktor.client.HttpClient
import io.ktor.client.call.body
import io.ktor.client.request.get
import io.ktor.client.statement.bodyAsChannel
import io.ktor.http.isSuccess
import io.ktor.utils.io.readAvailable
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.nio.file.attribute.PosixFilePermissions
import java.security.MessageDigest
import java.util.Comparator
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json

data class StagedUpdate(val manifest: UpdateManifest, val appPath: Path)

/** 签名、notarization、Team ID 和 SHA-256 四重校验的 macOS 更新器。 */
class JvmUpdateService(
    private val httpClient: HttpClient,
    private val paths: AppPaths,
    private val currentVersion: String,
    private val endpoint: String,
    private val expectedTeamId: String,
) {
    private val json = Json { ignoreUnknownKeys = true }

    suspend fun check(): UpdateManifest? {
        requireHttps(endpoint)
        val response = httpClient.get(endpoint)
        if (!response.status.isSuccess()) throw AppError.Remote(response.status.value, "检查更新失败")
        val manifest = json.decodeFromString<UpdateManifest>(response.body<String>())
        validateManifest(manifest)
        return manifest.takeIf { it.isNewerThan(currentVersion) }
    }

    suspend fun downloadAndStage(
        manifest: UpdateManifest,
        hasActiveTransfers: () -> Boolean,
        onProgress: (Long, Long?) -> Unit = { _, _ -> },
    ): StagedUpdate {
        validateManifest(manifest)
        if (expectedTeamId.isBlank()) throw AppError.Internal("正式更新未配置 Apple Team ID，拒绝安装")
        val idle = TransferIdleWaiter(hasActiveTransfers, System::currentTimeMillis, { delay(it) }).await()
        if (!idle) throw AppError.Internal("等待传输结束超过 5 分钟，已取消更新")

        val updateDir = paths.dataDir.resolve("updates").resolve(manifest.version)
        withContext(Dispatchers.IO) {
            deleteTree(updateDir)
            Files.createDirectories(updateDir)
        }
        val archive = updateDir.resolve("PetalLink.app.zip")
        download(manifest.url, archive, onProgress)
        val actualHash = withContext(Dispatchers.IO) { sha256(archive) }
        if (!actualHash.equals(manifest.sha256, ignoreCase = true)) {
            Files.deleteIfExists(archive)
            throw AppError.Data("更新包 SHA-256 不匹配")
        }

        val extracted = updateDir.resolve("extracted")
        Files.createDirectories(extracted)
        command("/usr/bin/ditto", "-x", "-k", archive.toString(), extracted.toString()).requireSuccess("解压更新包失败")
        val app = Files.list(extracted).use { files ->
            files.filter { it.fileName.toString().endsWith(".app") }.findFirst().orElse(null)
        } ?: throw AppError.Data("更新包中没有 .app")
        verifyApp(app)
        return StagedUpdate(manifest, app)
    }

    suspend fun launchInstaller(staged: StagedUpdate): Boolean = withContext(Dispatchers.IO) {
        val current = currentAppPath() ?: throw AppError.Internal("开发模式不能执行自更新安装")
        val parent = current.parent ?: throw AppError.LocalIo("无法定位应用目录")
        if (!Files.isWritable(parent)) throw AppError.LocalIo("应用目录不可写，请从 DMG 手动更新")
        val helper = paths.dataDir.resolve("updates").resolve("install-update.sh")
        Files.createDirectories(helper.parent)
        Files.writeString(helper, INSTALLER_SCRIPT, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING)
        runCatching { Files.setPosixFilePermissions(helper, PosixFilePermissions.fromString("rwx------")) }
        val backup = parent.resolve(".${current.fileName}.backup")
        val incoming = parent.resolve(".${current.fileName}.incoming")
        ProcessBuilder(
            "/bin/sh", helper.toString(), ProcessHandle.current().pid().toString(),
            current.toString(), staged.appPath.toString(), incoming.toString(), backup.toString(),
        ).redirectErrorStream(true).start()
        true
    }

    private suspend fun download(url: String, target: Path, onProgress: (Long, Long?) -> Unit) {
        requireHttps(url)
        val response = httpClient.get(url)
        if (!response.status.isSuccess()) throw AppError.Remote(response.status.value, "下载更新失败")
        val total = response.headers["Content-Length"]?.toLongOrNull()
        val part = target.resolveSibling("${target.fileName}.part")
        try {
            withContext(Dispatchers.IO) {
                Files.newOutputStream(part, StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING).use { output ->
                    val channel = response.bodyAsChannel()
                    val buffer = ByteArray(1024 * 1024)
                    var done = 0L
                    while (true) {
                        val count = channel.readAvailable(buffer)
                        if (count == -1) break
                        if (count == 0) continue
                        output.write(buffer, 0, count)
                        done += count
                        onProgress(done, total)
                    }
                }
                try {
                    Files.move(part, target, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
                } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
                    Files.move(part, target, StandardCopyOption.REPLACE_EXISTING)
                }
            }
        } finally {
            Files.deleteIfExists(part)
        }
    }

    private fun verifyApp(app: Path) {
        command("/usr/bin/codesign", "--verify", "--deep", "--strict", "--verbose=2", app.toString())
            .requireSuccess("更新应用代码签名无效")
        command("/usr/sbin/spctl", "--assess", "--type", "execute", "--verbose=2", app.toString())
            .requireSuccess("更新应用未通过 Gatekeeper/notarization")
        val details = command("/usr/bin/codesign", "-dv", "--verbose=4", app.toString())
        val team = Regex("(?m)^TeamIdentifier=(.+)$").find(details.output)?.groupValues?.get(1)?.trim()
        if (team != expectedTeamId) throw AppError.Data("更新应用 Team ID 不匹配")
    }

    private fun validateManifest(manifest: UpdateManifest) {
        if (SemanticVersion.parse(manifest.version) == null) throw AppError.Data("更新版本号无效")
        requireHttps(manifest.url)
        if (!manifest.sha256.matches(Regex("^[0-9a-fA-F]{64}$"))) throw AppError.Data("更新 SHA-256 无效")
        val minimum = parseSystemVersion(manifest.minimumSystemVersion)
            ?: throw AppError.Data("最低 macOS 版本无效")
        val current = parseSystemVersion(System.getProperty("os.version")) ?: minimum
        if (compareVersionParts(current, minimum) < 0) {
            throw AppError.Internal("此更新要求 macOS ${manifest.minimumSystemVersion} 或更高版本")
        }
    }

    private fun requireHttps(url: String) {
        if (!url.startsWith("https://")) throw AppError.Data("更新地址必须使用 HTTPS")
    }

    private fun currentAppPath(): Path? {
        val executable = ProcessHandle.current().info().command().orElse(null)?.let(Path::of) ?: return null
        val app = executable.parent?.parent?.parent ?: return null
        return app.takeIf { it.fileName.toString().endsWith(".app") && Files.isDirectory(it) }
    }

    private fun command(vararg args: String): CommandResult {
        val process = ProcessBuilder(args.toList()).redirectErrorStream(true).start()
        val output = process.inputStream.bufferedReader().readText()
        return CommandResult(process.waitFor(), output)
    }

    private fun sha256(path: Path): String {
        val digest = MessageDigest.getInstance("SHA-256")
        Files.newInputStream(path).use { input ->
            val buffer = ByteArray(1024 * 1024)
            while (true) {
                val count = input.read(buffer)
                if (count == -1) break
                digest.update(buffer, 0, count)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    private fun deleteTree(path: Path) {
        if (!Files.exists(path)) return
        Files.walk(path).use { files -> files.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
    }

    private fun parseSystemVersion(value: String): List<Int>? {
        val parts = value.substringBefore('-').split('.').map { it.toIntOrNull() ?: return null }.toMutableList()
        while (parts.size < 3) parts += 0
        return parts.take(3)
    }

    private fun compareVersionParts(left: List<Int>, right: List<Int>): Int {
        for (index in 0 until maxOf(left.size, right.size)) {
            val result = (left.getOrElse(index) { 0 }).compareTo(right.getOrElse(index) { 0 })
            if (result != 0) return result
        }
        return 0
    }

    private data class CommandResult(val exitCode: Int, val output: String) {
        fun requireSuccess(message: String) {
            if (exitCode != 0) throw AppError.Data("$message：${output.take(500)}")
        }
    }

    companion object {
        private val INSTALLER_SCRIPT = """#!/bin/sh
set -eu
pid="$1"
current="$2"
staged="$3"
incoming="$4"
backup="$5"
while kill -0 "${'$'}pid" 2>/dev/null; do sleep 1; done
rm -rf "${'$'}incoming" "${'$'}backup"
/usr/bin/ditto "${'$'}staged" "${'$'}incoming"
mv "${'$'}current" "${'$'}backup"
if mv "${'$'}incoming" "${'$'}current" && /usr/bin/open "${'$'}current"; then
  rm -rf "${'$'}backup"
  exit 0
fi
rm -rf "${'$'}current"
mv "${'$'}backup" "${'$'}current"
/usr/bin/open "${'$'}current" || true
exit 1
"""
    }
}
