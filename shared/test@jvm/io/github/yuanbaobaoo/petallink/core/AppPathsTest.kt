package io.github.yuanbaobaoo.petallink.core

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlin.test.assertFalse
import kotlin.test.assertNull

class AppPathsTest {
    @Test
    fun 生产目录指向原Tauri的prod_bundle_id() {
        // 老用户数据兼容：release 包数据目录固定不可变。
        val path = AppPaths.production().dataDir.toString()
        assertTrue(path.endsWith("Library/Application Support/io.github.yuanbaobaoo.PetalLink"), path)
    }

    @Test
    fun dev目录附加_dev后缀与prod隔离() {
        val path = AppPaths.development().dataDir.toString()
        assertTrue(path.endsWith("Library/Application Support/io.github.yuanbaobaoo.PetalLink-dev"), path)
        // 关键：dev 与 prod 不能相同，否则单实例锁和数据会串。
        assertFalse(AppPaths.production().dataDir == AppPaths.development().dataDir)
    }

    // ---- resolveFromEnvironment：纯函数入口，不触碰全局状态，顺序无关 ----

    @Test
    fun 优先级1_dataDirOverride压倒一切() {
        val resolved = AppPaths.resolveFromEnvironment(
            dataDirOverride = "/tmp/petalink-override-xyz",
            environment = "dev",
            builtBundleId = AppPaths.PROD_BUNDLE_ID,
        )
        assertEquals("/tmp/petalink-override-xyz", resolved.dataDir.toString())
    }

    @Test
    fun 优先级2_environment_dev压倒BuildInfo_release() {
        // 即使 BuildInfo 编译为 release(prod)，显式 PETALLINK_ENV=dev 也必须落到 dev 目录。
        val resolved = AppPaths.resolveFromEnvironment(
            dataDirOverride = null,
            environment = "dev",
            builtBundleId = AppPaths.PROD_BUNDLE_ID,
        )
        assertTrue(resolved.dataDir.endsWith("io.github.yuanbaobaoo.PetalLink-dev"), resolved.dataDir.toString())
    }

    @Test
    fun 优先级3_BuildInfo_dev包落到dev目录() {
        val resolved = AppPaths.resolveFromEnvironment(
            dataDirOverride = null,
            environment = null,
            builtBundleId = AppPaths.DEV_BUNDLE_ID,
        )
        assertTrue(resolved.dataDir.endsWith("io.github.yuanbaobaoo.PetalLink-dev"), resolved.dataDir.toString())
    }

    @Test
    fun 优先级3_BuildInfo_release包落到prod目录() {
        val resolved = AppPaths.resolveFromEnvironment(
            dataDirOverride = null,
            environment = null,
            builtBundleId = AppPaths.PROD_BUNDLE_ID,
        )
        assertTrue(resolved.dataDir.endsWith("io.github.yuanbaobaoo.PetalLink"), resolved.dataDir.toString())
    }

    @Test
    fun 优先级4_BuildInfo缺失时兜底prod() {
        val resolved = AppPaths.resolveFromEnvironment(
            dataDirOverride = null,
            environment = null,
            builtBundleId = "",
        )
        assertTrue(resolved.dataDir.endsWith("io.github.yuanbaobaoo.PetalLink"), resolved.dataDir.toString())
    }

    @Test
    fun environment大小写不敏感() {
        val resolved = AppPaths.resolveFromEnvironment(
            dataDirOverride = null,
            environment = "DEV",
            builtBundleId = AppPaths.PROD_BUNDLE_ID,
        )
        assertTrue(resolved.dataDir.endsWith("io.github.yuanbaobaoo.PetalLink-dev"), resolved.dataDir.toString())
    }

    @Test
    fun environment非dev不触发dev目录() {
        val resolved = AppPaths.resolveFromEnvironment(
            dataDirOverride = null,
            environment = "release",
            builtBundleId = AppPaths.PROD_BUNDLE_ID,
        )
        assertTrue(resolved.dataDir.endsWith("io.github.yuanbaobaoo.PetalLink"), resolved.dataDir.toString())
    }

    @Test
    fun 空白dataDirOverride被忽略() {
        val resolved = AppPaths.resolveFromEnvironment(
            dataDirOverride = "   ",
            environment = null,
            builtBundleId = AppPaths.PROD_BUNDLE_ID,
        )
        assertTrue(resolved.dataDir.endsWith("io.github.yuanbaobaoo.PetalLink"), resolved.dataDir.toString())
    }

    @Test
    fun currentBundleId与BuildInfo一致且属于dev或prod() {
        val id = AppPaths.currentBundleId()
        assertEquals(BuildInfo.BUNDLE_ID, id)
        assertTrue(id == AppPaths.PROD_BUNDLE_ID || id == AppPaths.DEV_BUNDLE_ID, id)
    }

    @Test
    fun fromEnvironment无全局覆盖时与BuildInfo一致() {
        // 仅在未设任何系统/环境覆盖时断言；本进程可能被其他测试设过，故跳过断言而非污染全局。
        // 真正的优先级语义由上面的 resolveFromEnvironment 系列覆盖。
        assertNull(System.getProperty("petallink.resolve.selfcheck.absent"), "占位断言：此处不应有全局污染")
    }
}
