package io.github.yuanbaobaoo.petallink.mount

import java.nio.file.Path
import java.nio.file.Files
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.launch

/** FSEvents 递归监听 + 2s warmup + 3s debounce + generation 隔离。 */
class JvmLocalWatcher(
    mountRoot: Path,
    private val scope: CoroutineScope,
    private val skipPatterns: List<String> = SkipFilter.DEFAULT_PATTERNS,
    private val sourceFactory: FSEventSourceFactory = MacFSEventSourceFactory,
    private val debounceMs: Long = 3_000,
    private val warmupMs: Long = 2_000,
    private val nanoTime: () -> Long = System::nanoTime,
) : AutoCloseable {
    private val lexicalRoot = mountRoot.toAbsolutePath().normalize().also {
        require(!Files.isSymbolicLink(it) && Files.isDirectory(it)) { "FSEvents 根路径必须是非符号链接目录: $it" }
    }
    private val root = lexicalRoot.toRealPath()
    private val generation = AtomicLong(0)
    private val lock = Any()
    private val pending = linkedSetOf<String>()
    private var source: AutoCloseable? = null
    private var debounceJob: Job? = null
    private var warmupJob: Job? = null
    private var warmupUntilNanos = 0L
    private val mutableChanges = MutableSharedFlow<List<String>>(extraBufferCapacity = 64)
    val changes: SharedFlow<List<String>> = mutableChanges.asSharedFlow()

    fun start() {
        stop()
        val current = generation.incrementAndGet()
        warmupUntilNanos = nanoTime() + warmupMs * 1_000_000
        val created = sourceFactory.start(listOf(root.toString())) { event ->
            handleEvent(current, event)
        }
        synchronized(lock) { source = created }
        warmupJob = scope.launch {
            delay(warmupMs)
            if (generation.get() == current) mutableChanges.emit(emptyList())
        }
    }

    fun stop() {
        generation.incrementAndGet()
        val oldSource: AutoCloseable?
        synchronized(lock) {
            oldSource = source
            source = null
            pending.clear()
        }
        debounceJob?.cancel()
        debounceJob = null
        warmupJob?.cancel()
        warmupJob = null
        runCatching { oldSource?.close() }
    }

    private fun handleEvent(current: Long, event: NativeFSEvent) {
        if (generation.get() != current || nanoTime() < warmupUntilNanos) return
        val relative = sanitize(event.path) ?: return
        synchronized(lock) {
            if (generation.get() != current) return
            pending += relative
            debounceJob?.cancel()
            debounceJob = scope.launch {
                delay(debounceMs)
                val batch = synchronized(lock) {
                    if (generation.get() != current) return@launch
                    pending.toList().also { pending.clear() }
                }
                if (batch.isNotEmpty() && generation.get() == current) mutableChanges.emit(batch.sorted())
            }
        }
    }

    private fun sanitize(rawPath: String): String? {
        val path = runCatching { Path.of(rawPath).toAbsolutePath().normalize() }.getOrNull() ?: return null
        val eventRoot = when {
            path.startsWith(root) -> root
            path.startsWith(lexicalRoot) -> lexicalRoot
            else -> return null
        }
        val relative = eventRoot.relativize(path)
        if (relative.none()) return ""
        if (relative.any { SkipFilter.shouldSkip(it.toString(), skipPatterns) }) return null
        return relative.joinToString("/") { it.toString() }
    }

    override fun close() = stop()
}
