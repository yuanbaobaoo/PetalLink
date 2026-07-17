package io.github.yuanbaobaao.petallink.core

import java.nio.file.Path
import java.nio.file.Paths

/** 应用运行文件的唯一入口。生产与 dev/test 目录由 Composition Root 明确选择。 */
data class AppPaths(val dataDir: Path) {
    val configFile: Path get() = dataDir.resolve("config.json")
    val databaseFile: Path get() = dataDir.resolve("petal_link.db")
    val tokenFile: Path get() = dataDir.resolve("token.bin")
    val logsDir: Path get() = dataDir.resolve("logs")

    fun cloudTreeCheckpoint(mountRoot: Path): Path {
        val absolute = mountRoot.toAbsolutePath().normalize().toString()
        val escaped = buildString(absolute.length) {
            absolute.forEach { char ->
                append(if (char.isLetterOrDigit() && char.code < 128 || char == '.' || char == '_' || char == '-') char else '_')
            }
        }
        return dataDir.resolve("cloudtree_$escaped.json")
    }

    companion object {
        const val PROD_BUNDLE_ID = "io.github.yuanbaobaoo.PetalLink"
        const val DEV_BUNDLE_ID = "$PROD_BUNDLE_ID-dev"

        fun production(): AppPaths = fromBundleId(PROD_BUNDLE_ID)
        fun development(): AppPaths = fromBundleId(DEV_BUNDLE_ID)

        /** `petallink.dataDir` 优先，便于测试和本地开发完全隔离。 */
        fun fromEnvironment(): AppPaths {
            val override = System.getProperty("petallink.dataDir")
                ?.takeIf { it.isNotBlank() }
                ?: System.getenv("PETALLINK_DATA_DIR")?.takeIf { it.isNotBlank() }
            if (override != null) return AppPaths(Paths.get(override).toAbsolutePath().normalize())
            val environment = System.getProperty("petallink.environment")
                ?: System.getenv("PETALLINK_ENV")
            return if (environment.equals("dev", ignoreCase = true)) development() else production()
        }

        private fun fromBundleId(bundleId: String): AppPaths = AppPaths(
            Paths.get(System.getProperty("user.home"), "Library", "Application Support", bundleId),
        )
    }
}
