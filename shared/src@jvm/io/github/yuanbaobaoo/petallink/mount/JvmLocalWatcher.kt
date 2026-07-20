package io.github.yuanbaobaoo.petallink.mount

import io.github.yuanbaobaoo.petallink.core.logging.Logger
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

/**
 * FSEvents 递归监听 + 2s warmup + 3s debounce + generation 隔离。
 */
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
    private val logger = Logger()
    private val generation = AtomicLong(0)
    private val lock = Any()
    private val pending = linkedSetOf<String>()
    private var source: AutoCloseable? = null
    private var debounceJob: Job? = null
    private var warmupJob: Job? = null
    private var warmupUntilNanos = 0L
    private val mutableChanges = MutableSharedFlow<List<String>>(extraBufferCapacity = 64)
    val changes: SharedFlow<List<String>> = mutableChanges.asSharedFlow()

    /**
     * 启动 FSEvents 监听：先停止旧代际，进入 warmup 后开始接收事件。
     */
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
        logger.info("mount.local_watcher") { "本地文件监视器已启动 dir=$root debounce=${debounceMs / 1_000}" }
    }

    /**
     * 停止监听：失效当前代际、清理待处理事件、取消 debounce/warmup 任务并关闭原生事件源。
     */
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
        if (oldSource != null) {
            logger.info("mount.local_watcher") { "本地文件监视器已停止" }
        }
    }

    /**
     * 处理单条原生事件：纯元数据事件（如 xattr 写入）直接忽略，warmup 与代际失配直接忽略，命中则加入待处理集合并启动 debounce。
     */
    private fun handleEvent(current: Long, event: NativeFSEvent) {
        // 仅放行内容级变更（对标 local_watcher.rs:307-316 的 Create/Modify/Remove/Other 过滤）；
        // 占位符创建与 markDownloaded 写 xattr 只产生元数据位事件，放行会自触发全量重扫。
        if (!event.isChangeEvent()) {
            logger.debug("mount.local_watcher") { "watcher: 忽略纯元数据事件 kind=${event.flags} path=${event.path}" }
            return
        }
        if (generation.get() != current) return
        if (nanoTime() < warmupUntilNanos) {
            logger.debug("mount.local_watcher") { "watcher warmup: 丢弃历史事件 kind=${event.flags}" }
            return
        }
        val relative = sanitize(event.path) ?: return
        logger.debug("mount.local_watcher") { "watcher: 接受事件 kind=${event.flags} paths=[$relative]" }
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

    /**
     * 将原生事件路径规范化为相对挂载根的路径字符串，越界或命中跳过模式时返回 null。
     */
    private fun sanitize(rawPath: String): String? {
        val path = runCatching { Path.of(rawPath).toAbsolutePath().normalize() }.getOrNull() ?: return null
        val eventRoot = when {
            path.startsWith(root) -> root
            path.startsWith(lexicalRoot) -> lexicalRoot
            else -> {
                logger.debug("mount.local_watcher") { "watcher: 路径不在挂载目录下，跳过 path=$path mount=$root" }
                return null
            }
        }
        val relative = eventRoot.relativize(path)
        if (relative.none()) return ""
        if (relative.any { SkipFilter.shouldSkip(it.toString(), skipPatterns) }) {
            logger.debug("mount.local_watcher") { "watcher: 跳过排除文件 path=$path" }
            return null
        }
        return relative.joinToString("/") { it.toString() }
    }

    /**
     * 关闭监听器，等价于 [stop]。
     */
    override fun close() = stop()
}
