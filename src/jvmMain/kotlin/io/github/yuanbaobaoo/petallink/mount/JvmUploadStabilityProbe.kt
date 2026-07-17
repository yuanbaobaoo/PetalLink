package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.sync.engine.UploadStability
import io.github.yuanbaobaoo.petallink.sync.engine.UploadStabilityProbe
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.delay

/**
 * mtime 5s + size 3s + lsof 双重复核，并追踪 5min 持续编辑。
 */
class JvmUploadStabilityProbe(
    private val busyChecker: LsofFileBusyChecker = LsofFileBusyChecker(),
    private val nowMs: () -> Long = System::currentTimeMillis,
    private val pause: suspend (Long) -> Unit = { delay(it) },
) : UploadStabilityProbe {
    private val firstUnstable = ConcurrentHashMap<String, Long>()

    /**
     * 通过 mtime、size 间隔复核与 lsof 占用检查判定上传稳定性。
     */
    override suspend fun check(path: String): UploadStability {
        val target = Path.of(path).toAbsolutePath().normalize()
        if (Files.isSymbolicLink(target) || !Files.isRegularFile(target, LinkOption.NOFOLLOW_LINKS)) {
            return unstable(path)
        }
        val before = Files.readAttributes(target, "basic:size,lastModifiedTime", LinkOption.NOFOLLOW_LINKS)
        val modified = (before["lastModifiedTime"] as java.nio.file.attribute.FileTime).toMillis()
        if (nowMs() - modified < 5_000) return unstable(path)
        val size = before["size"] as Long
        pause(3_000)
        val after = Files.readAttributes(target, "basic:size,lastModifiedTime", LinkOption.NOFOLLOW_LINKS)
        if (after["size"] != size || (after["lastModifiedTime"] as java.nio.file.attribute.FileTime).toMillis() != modified) {
            return unstable(path)
        }
        if (busyChecker.check(target).busy) return unstable(path)
        firstUnstable.remove(path)
        return UploadStability.STABLE
    }

    /**
     * 记录并返回不稳定状态；持续不稳定超过 5 分钟则升级为 EDITING。
     */
    private fun unstable(path: String): UploadStability {
        val now = nowMs()
        val first = firstUnstable.putIfAbsent(path, now) ?: now
        return if (now - first >= 300_000) UploadStability.EDITING else UploadStability.UNSTABLE
    }
}
