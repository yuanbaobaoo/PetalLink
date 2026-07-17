package io.github.yuanbaobaoo.petallink.core

import java.nio.file.Path
import java.nio.file.Paths

/**
 * 应用运行文件的唯一入口。生产与 dev/test 目录由 Composition Root 明确选择。
 *
 * bundle id 是数据目录、单实例锁和 LaunchAgent label 的共同真相源：
 * - 打包时由 gradle 属性 `petalLinkBuildProfile` 决定写入 .app 的 CFBundleIdentifier，
 *   并以同名常量编译进 [BuildInfo.BUNDLE_ID]；
 * - 运行时 [fromEnvironment] 默认读取 [BuildInfo.BUNDLE_ID]，使 dev 包自动落到 dev
 *   数据目录、release 包自动落到 prod 数据目录，二者彻底隔离。
 *
 * 优先级（高 → 低）：
 * 1. `petallink.dataDir` 系统属性 / `PETALLINK_DATA_DIR` 环境变量 —— 测试与本地隔离；
 * 2. `petallink.environment=dev` / `PETALLINK_ENV=dev` —— 显式覆盖（如未打包的 gradle run）；
 * 3. [BuildInfo.BUNDLE_ID] —— 打包 app 的默认真相源；
 * 4. [PROD_BUNDLE_ID] —— 兜底（BuildInfo 缺失时，例如旧 classpath）。
 */
data class AppPaths(val dataDir: Path) {
    val configFile: Path get() = dataDir.resolve("config.json")
    val databaseFile: Path get() = dataDir.resolve("petal_link.db")
    val tokenFile: Path get() = dataDir.resolve("token.bin")
    val logsDir: Path get() = dataDir.resolve("logs")

    /**
     * 根据挂载根路径计算对应的可信云树检查点文件路径（路径被转义为安全文件名片段）。
     */
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

        /**
         * 返回生产 bundle id 对应的应用路径。
         */
        fun production(): AppPaths = fromBundleId(PROD_BUNDLE_ID)
        /**
         * 返回开发 bundle id 对应的应用路径。
         */
        fun development(): AppPaths = fromBundleId(DEV_BUNDLE_ID)

        /**
         * 当前运行时生效的 bundle id（与 LaunchAgent label、数据目录一致）。
         */
        fun currentBundleId(): String {
            // BuildInfo.BUNDLE_ID 为空串仅在异常构建场景出现，兜底 prod。
            val built = runCatching { BuildInfo.BUNDLE_ID }.getOrDefault("")
            return built.takeIf { it.isNotBlank() } ?: PROD_BUNDLE_ID
        }

        /**
         * `petallink.dataDir` / `PETALLINK_DATA_DIR` 优先，便于测试和本地开发完全隔离。
         */
        fun fromEnvironment(): AppPaths = resolveFromEnvironment(
            dataDirOverride = System.getProperty("petallink.dataDir")
                ?.takeIf { it.isNotBlank() }
                ?: System.getenv("PETALLINK_DATA_DIR")?.takeIf { it.isNotBlank() },
            environment = System.getProperty("petallink.environment") ?: System.getenv("PETALLINK_ENV"),
            builtBundleId = runCatching { BuildInfo.BUNDLE_ID }.getOrDefault(""),
        )

        /**
         * 优先级解析的纯函数入口；不读取任何全局状态，便于确定性测试。
         * 优先级（高 → 低）：[dataDirOverride] → [environment]==dev → [builtBundleId] → prod 兜底。
         */
        internal fun resolveFromEnvironment(
            dataDirOverride: String?,
            environment: String?,
            builtBundleId: String,
        ): AppPaths {
            if (!dataDirOverride.isNullOrBlank()) {
                return AppPaths(Paths.get(dataDirOverride).toAbsolutePath().normalize())
            }
            if (environment.equals("dev", ignoreCase = true)) return development()
            // 打包 app 的默认真相源：BuildInfo.BUNDLE_ID（dev 包→dev 目录，release 包→prod 目录）。
            if (builtBundleId.isNotBlank()) return fromBundleId(builtBundleId)
            // 兜底：未打包且未显式指定环境时按 prod。
            return production()
        }

        /**
         * 按 bundle id 拼接出 `~/Library/Application Support/<bundleId>` 数据目录。
         */
        internal fun fromBundleId(bundleId: String): AppPaths = AppPaths(
            Paths.get(System.getProperty("user.home"), "Library", "Application Support", bundleId),
        )
    }
}
