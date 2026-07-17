package io.github.yuanbaobaoo.petallink.mount

import com.sun.jna.Callback
import com.sun.jna.Library
import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Platform
import com.sun.jna.Pointer
import io.github.yuanbaobaoo.petallink.AppError
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import java.nio.file.Path

/**
 * FSEvents 原生事件，包含变更路径、事件标志位和事件 ID。
 */
data class NativeFSEvent(val path: String, val flags: Int, val eventId: ULong)

/**
 * FSEvents 事件源工厂，创建对指定路径的监听并返回可关闭的资源
 */
fun interface FSEventSourceFactory {
    /**
     * 启动对指定路径的监听，事件经 [callback] 回调；返回的 [AutoCloseable] 用于停止监听并释放资源。
     */
    fun start(paths: List<String>, callback: (NativeFSEvent) -> Unit): AutoCloseable
}

/**
 * CoreServices FSEventStream 的 JNA 封装；回调由专用 CFRunLoop daemon 线程承载。
 */
object MacFSEventSourceFactory : FSEventSourceFactory {
    /**
     * 启动 macOS FSEvents 监听，返回可关闭的 [MacFSEventSource]。
     */
    override fun start(paths: List<String>, callback: (NativeFSEvent) -> Unit): AutoCloseable =
        MacFSEventSource(
            paths.map { raw -> Path.of(raw).toRealPath().toString() },
            callback,
        ).also(MacFSEventSource::start)
}

/**
 * macOS FSEvents 监听源实现：基于 JNA 调用 CoreServices/CoreFoundation 启动事件流，
 * 在专用 CFRunLoop 线程上接收并派发原生事件。
 */
private class MacFSEventSource(
    private val paths: List<String>,
    private val consumer: (NativeFSEvent) -> Unit,
) : AutoCloseable {
    /**
     * CoreFoundation 框架的 JNA 绑定接口（CFString / CFArray / CFRunLoop 等引用计数与运行循环管理）。
     */
    private interface CoreFoundation : Library {
        /**
         * CoreFoundation CFStringCreateWithCString 的 JNA 声明，按指定编码创建 CFString。
         */
        fun CFStringCreateWithCString(allocator: Pointer?, value: String, encoding: Int): Pointer?
        /**
         * CoreFoundation CFArrayCreate 的 JNA 声明，由一组指针构造不可变 CFArray。
         */
        fun CFArrayCreate(allocator: Pointer?, values: Pointer, count: Long, callbacks: Pointer?): Pointer?
        /**
         * CoreFoundation CFRunLoopGetCurrent 的 JNA 声明，取当前线程的 CFRunLoop。
         */
        fun CFRunLoopGetCurrent(): Pointer?
        /**
         * CoreFoundation CFRunLoopRun 的 JNA 声明，在当前线程阻塞运行 CFRunLoop。
         */
        fun CFRunLoopRun()
        /**
         * CoreFoundation CFRunLoopStop 的 JNA 声明，停止指定 CFRunLoop。
         */
        fun CFRunLoopStop(runLoop: Pointer)
        /**
         * CoreFoundation CFRelease 的 JNA 声明，释放 CoreFoundation 对象的引用计数。
         */
        fun CFRelease(value: Pointer)
    }

    /**
     * CoreServices 框架（FSEvents）的 JNA 绑定接口，用于创建与控制文件系统事件流。
     */
    private interface CoreServices : Library {
        /**
         * CoreServices FSEventStreamCreate 的 JNA 声明，创建一个监听路径集的 FSEventStream。
         */
        fun FSEventStreamCreate(
            allocator: Pointer?,
            callback: FSEventCallback,
            context: Pointer?,
            pathsToWatch: Pointer,
            sinceWhen: Long,
            latency: Double,
            flags: Int,
        ): Pointer?
        /**
         * CoreServices FSEventStreamSetDispatchQueue 的 JNA 声明，指定事件派发的 GCD 队列。
         */
        fun FSEventStreamSetDispatchQueue(stream: Pointer, queue: Pointer)
        /**
         * CoreServices FSEventStreamStart 的 JNA 声明，开始接收事件；返回非 0 表示成功。
         */
        fun FSEventStreamStart(stream: Pointer): Byte
        /**
         * CoreServices FSEventStreamStop 的 JNA 声明，停止接收事件。
         */
        fun FSEventStreamStop(stream: Pointer)
        /**
         * CoreServices FSEventStreamInvalidate 的 JNA 声明，作废流并解除资源绑定。
         */
        fun FSEventStreamInvalidate(stream: Pointer)
        /**
         * CoreServices FSEventStreamRelease 的 JNA 声明，释放流对象的引用计数。
         */
        fun FSEventStreamRelease(stream: Pointer)
    }

    /**
     * libdispatch 的 JNA 绑定接口，用于获取 GCD 全局并发队列以派发 FSEvents 回调。
     */
    private interface Dispatch : Library {
        /**
         * libdispatch dispatch_get_global_queue 的 JNA 声明，按优先级获取系统全局并发队列。
         */
        fun dispatch_get_global_queue(identifier: Long, flags: Long): Pointer?
    }

    /**
     * FSEventStream 回调，由 native 层在事件到达时调用
     */
    private fun interface FSEventCallback : Callback {
        /**
         * FSEventStream 回调签名，由 native 层在事件到达时调用。
         */
        fun invoke(
            streamRef: Pointer,
            clientInfo: Pointer?,
            numEvents: Long,
            eventPaths: Pointer,
            eventFlags: Pointer,
            eventIds: Pointer,
        )
    }

    private val cf: CoreFoundation by lazy { Native.load("CoreFoundation", CoreFoundation::class.java) }
    private val fs: CoreServices by lazy { Native.load("CoreServices", CoreServices::class.java) }
    private val dispatch: Dispatch by lazy { Native.load(Platform.C_LIBRARY_NAME, Dispatch::class.java) }
    private val closed = AtomicBoolean(false)
    private val closeSignal = CountDownLatch(1)
    private val started = CountDownLatch(1)
    private val startupError = AtomicReference<Throwable?>()
    @Volatile private var stream: Pointer? = null
    private lateinit var thread: Thread

    // 必须强引用，防止 native stream 存活期间 callback 被 GC。
    private val nativeCallback = FSEventCallback { _, _, count, eventPaths, eventFlags, eventIds ->
        for (index in 0 until count.coerceAtMost(Int.MAX_VALUE.toLong()).toInt()) {
            val pathPointer = eventPaths.getPointer(index.toLong() * Native.POINTER_SIZE)
            val path = pathPointer?.getString(0, Charsets.UTF_8.name()) ?: continue
            val flags = eventFlags.getInt(index.toLong() * Int.SIZE_BYTES)
            val eventId = eventIds.getLong(index.toLong() * Long.SIZE_BYTES).toULong()
            runCatching { consumer(NativeFSEvent(path, flags, eventId)) }
        }
    }

    /**
     * 启动 FSEvents 监听线程，等待流创建成功或失败后返回；超时/失败时会清理并抛出异常。
     */
    fun start() {
        if (!Platform.isMac()) throw AppError.LocalIo("FSEvents 仅支持 macOS")
        require(paths.isNotEmpty()) { "FSEvents 监听路径不能为空" }
        thread = Thread(::runLoopMain, "petallink-fsevents").apply {
            isDaemon = true
            start()
        }
        if (!started.await(5, TimeUnit.SECONDS)) {
            close()
            throw AppError.LocalIo("FSEvents 启动超时")
        }
        startupError.get()?.let {
            close()
            throw AppError.LocalIo("FSEvents 启动失败", it)
        }
    }

    /**
     * daemon 线程主体：组装路径 CFArray、创建并启动 FSEventStream，随后阻塞直至收到关闭信号，退出时释放所有 native 资源。
     */
    private fun runLoopMain() {
        val strings = mutableListOf<Pointer>()
        var array: Pointer? = null
        try {
            paths.forEach { path ->
                strings += cf.CFStringCreateWithCString(null, path, CF_STRING_ENCODING_UTF8)
                    ?: error("CFStringCreateWithCString 失败: $path")
            }
            val pointers = Memory(paths.size.toLong() * Native.POINTER_SIZE)
            strings.forEachIndexed { index, value ->
                pointers.setPointer(index.toLong() * Native.POINTER_SIZE, value)
            }
            array = cf.CFArrayCreate(null, pointers, strings.size.toLong(), null)
                ?: error("CFArrayCreate 失败")
            val created = fs.FSEventStreamCreate(
                null,
                nativeCallback,
                null,
                array,
                -1L,
                0.1,
                FLAG_NO_DEFER or FLAG_WATCH_ROOT or FLAG_FILE_EVENTS,
            ) ?: error("FSEventStreamCreate 失败")
            stream = created
            val queue = dispatch.dispatch_get_global_queue(0, 0)
                ?: error("dispatch_get_global_queue 失败")
            fs.FSEventStreamSetDispatchQueue(created, queue)
            if (fs.FSEventStreamStart(created).toInt() == 0) error("FSEventStreamStart 失败")
            started.countDown()
            if (!closed.get()) closeSignal.await()
        } catch (error: Throwable) {
            startupError.compareAndSet(null, error)
            started.countDown()
        } finally {
            stream?.let { value ->
                runCatching { fs.FSEventStreamStop(value) }
                runCatching { fs.FSEventStreamInvalidate(value) }
                runCatching { fs.FSEventStreamRelease(value) }
            }
            stream = null
            array?.let { runCatching { cf.CFRelease(it) } }
            strings.forEach { runCatching { cf.CFRelease(it) } }
        }
    }

    /**
     * 停止流并唤醒 daemon 线程，等待其完成 native 资源释放。
     */
    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        stream?.let { runCatching { fs.FSEventStreamStop(it) } }
        closeSignal.countDown()
        if (::thread.isInitialized && thread !== Thread.currentThread()) thread.join(5_000)
    }

    companion object {
        private const val CF_STRING_ENCODING_UTF8 = 0x08000100
        private const val FLAG_NO_DEFER = 0x00000002
        private const val FLAG_WATCH_ROOT = 0x00000004
        private const val FLAG_FILE_EVENTS = 0x00000010
    }
}
