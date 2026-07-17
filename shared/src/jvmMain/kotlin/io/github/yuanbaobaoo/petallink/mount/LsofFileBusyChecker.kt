package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.config.AppConfig
import java.nio.file.Path
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

object LsofParser {
    /** 解析 `lsof -F pc` 输出，只接受附属于已见 pid 记录的 command。 */
    fun commands(output: String): List<String> {
        val result = linkedSetOf<String>()
        var hasProcess = false
        output.lineSequence().forEach { line ->
            when {
                line.startsWith('p') -> hasProcess = line.drop(1).toLongOrNull() != null
                hasProcess && line.startsWith('c') -> line.drop(1).takeIf(String::isNotBlank)?.let(result::add)
            }
        }
        return result.toList()
    }
}

fun interface LsofSampler {
    fun sample(path: Path): List<String>
}

data class BusyCheck(val busy: Boolean, val processes: List<String>)

/** macOS lsof 采样与 1 秒二次确认。 */
class LsofFileBusyChecker(
    private val sampler: LsofSampler = LsofSampler(::sampleWithLsof),
    private val pause: suspend (Long) -> Unit = { delay(it) },
    private val doubleCheckMs: Long = AppConfig.STABILITY_LSOF_DOUBLE_CHECK_SECS * 1_000L,
) {
    suspend fun check(path: Path): BusyCheck = withContext(Dispatchers.IO) {
        val first = nonWhitelisted(sampler.sample(path))
        if (first.isEmpty()) return@withContext BusyCheck(false, emptyList())
        pause(doubleCheckMs)
        val second = nonWhitelisted(sampler.sample(path))
        BusyCheck(second.isNotEmpty(), second)
    }

    private fun nonWhitelisted(commands: List<String>): List<String> =
        commands.filterNot { it in AppConfig.STABILITY_LSOF_WHITELIST }.distinct()

    companion object {
        fun sampleWithLsof(path: Path): List<String> {
            val process = try {
                ProcessBuilder("lsof", "-nP", "-F", "pc", path.toAbsolutePath().normalize().toString())
                    .redirectErrorStream(true)
                    .start()
            } catch (error: Throwable) {
                throw AppError.LocalIo("无法启动 lsof", error)
            }
            if (!process.waitFor(5, TimeUnit.SECONDS)) {
                process.destroyForcibly()
                throw AppError.LocalIo("lsof 超时: $path")
            }
            val output = process.inputStream.bufferedReader().use { it.readText() }
            return when (process.exitValue()) {
                0 -> LsofParser.commands(output)
                1 -> emptyList() // lsof 未找到打开者的标准退出码
                else -> throw AppError.LocalIo("lsof 失败 exit=${process.exitValue()}: ${output.take(500)}")
            }
        }
    }
}
