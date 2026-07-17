package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.AppError
import io.github.yuanbaobaoo.petallink.config.AppConfig
import java.nio.file.Path
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/**
 * lsof 输出解析器，提取占用文件的进程命令名。
 */
object LsofParser {
    /**
     * 解析 `lsof -F pc` 输出，只接受附属于已见 pid 记录的 command。
     */
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

/**
 * lsof 采样接口，抽象为函数式接口便于测试时替换实现
 */
fun interface LsofSampler {
    /**
     * 采样占用指定文件的进程命令名列表。
     */
    fun sample(path: Path): List<String>
}

/**
 * 文件占用检查结果；busy 表示文件仍被占用，processes 为占用进程命令名列表。
 */
data class BusyCheck(val busy: Boolean, val processes: List<String>)

/**
 * macOS lsof 采样与 1 秒二次确认。
 */
class LsofFileBusyChecker(
    private val sampler: LsofSampler = LsofSampler(::sampleWithLsof),
    private val pause: suspend (Long) -> Unit = { delay(it) },
    private val doubleCheckMs: Long = AppConfig.STABILITY_LSOF_DOUBLE_CHECK_SECS * 1_000L,
) {
    /**
     * 采样占用进程并在间隔后二次确认，仅当两次均存在非白名单进程时判定为 busy。
     */
    suspend fun check(path: Path): BusyCheck = withContext(Dispatchers.IO) {
        val first = nonWhitelisted(sampler.sample(path))
        if (first.isEmpty()) return@withContext BusyCheck(false, emptyList())
        pause(doubleCheckMs)
        val second = nonWhitelisted(sampler.sample(path))
        BusyCheck(second.isNotEmpty(), second)
    }

    /**
     * 过滤掉配置白名单中的进程命令并去重。
     */
    private fun nonWhitelisted(commands: List<String>): List<String> =
        commands.filterNot { it in AppConfig.STABILITY_LSOF_WHITELIST }.distinct()

    companion object {
        /**
         * 调用系统 lsof 采样占用文件的进程命令名，超时或异常转换为 AppError.LocalIo。
         */
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
