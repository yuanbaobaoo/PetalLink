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

data class NativeFSEvent(val path: String, val flags: Int, val eventId: ULong)

fun interface FSEventSourceFactory {
    fun start(paths: List<String>, callback: (NativeFSEvent) -> Unit): AutoCloseable
}

/** CoreServices FSEventStream 的 JNA 封装；回调由专用 CFRunLoop daemon 线程承载。 */
object MacFSEventSourceFactory : FSEventSourceFactory {
    override fun start(paths: List<String>, callback: (NativeFSEvent) -> Unit): AutoCloseable =
        MacFSEventSource(
            paths.map { raw -> Path.of(raw).toRealPath().toString() },
            callback,
        ).also(MacFSEventSource::start)
}

private class MacFSEventSource(
    private val paths: List<String>,
    private val consumer: (NativeFSEvent) -> Unit,
) : AutoCloseable {
    private interface CoreFoundation : Library {
        fun CFStringCreateWithCString(allocator: Pointer?, value: String, encoding: Int): Pointer?
        fun CFArrayCreate(allocator: Pointer?, values: Pointer, count: Long, callbacks: Pointer?): Pointer?
        fun CFRunLoopGetCurrent(): Pointer?
        fun CFRunLoopRun()
        fun CFRunLoopStop(runLoop: Pointer)
        fun CFRelease(value: Pointer)
    }

    private interface CoreServices : Library {
        fun FSEventStreamCreate(
            allocator: Pointer?,
            callback: FSEventCallback,
            context: Pointer?,
            pathsToWatch: Pointer,
            sinceWhen: Long,
            latency: Double,
            flags: Int,
        ): Pointer?
        fun FSEventStreamSetDispatchQueue(stream: Pointer, queue: Pointer)
        fun FSEventStreamStart(stream: Pointer): Byte
        fun FSEventStreamStop(stream: Pointer)
        fun FSEventStreamInvalidate(stream: Pointer)
        fun FSEventStreamRelease(stream: Pointer)
    }

    private interface Dispatch : Library {
        fun dispatch_get_global_queue(identifier: Long, flags: Long): Pointer?
    }

    private fun interface FSEventCallback : Callback {
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
