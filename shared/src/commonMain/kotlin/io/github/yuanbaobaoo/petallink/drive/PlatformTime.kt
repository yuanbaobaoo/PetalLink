package io.github.yuanbaobaoo.petallink.drive

/**
 * 平台时间工具（expect，macosMain 提供 actual）。
 */
expect object PlatformTime {
    /** 当前微秒时间戳（boundary 用） */
    fun micros(): Long

    /** 当前毫秒时间戳 */
    fun millis(): Long
}
