package io.github.yuanbaobaao.petallink.drive

/** JVM 平台时间实现 */
actual object PlatformTime {
    actual fun micros(): Long = System.currentTimeMillis() * 1000L

    actual fun millis(): Long = System.currentTimeMillis()
}
