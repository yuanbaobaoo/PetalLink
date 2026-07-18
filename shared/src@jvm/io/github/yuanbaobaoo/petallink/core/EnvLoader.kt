package io.github.yuanbaobaoo.petallink.core

/**
 * 凭据加载（对标 src/core/env_loader.rs + constants.rs 解析优先级）。
 *
 * 优先级：编译期 env > 运行时 .env > 占位符（空/REPLACE_WITH_REAL_SECRET）。
 * 详见 docs/07 §凭据。
 */

object EnvLoader {

    /**
     * 华为 OAuth client credentials（由 .env 或环境变量注入）
     */
    private var envClientId: String? = System.getenv("HWCLOUD_CLIENT_ID")
    private var envClientSecret: String? = System.getenv("HWCLOUD_CLIENT_SECRET")

    /**
     * 占位符常量
     */
    const val PLACEHOLDER_SECRET = "REPLACE_WITH_REAL_SECRET"

    /**
     * 构建时注入的凭据（由外部设置）
     */
    var buildClientId: String = ""
    var buildSecret: String = ""

    /**
     * 从 .env 文件加载凭据。
     *
     * 搜索顺序（首个命中即用）：
     * 1. 当前工作目录的 `.env`
     * 2. 当前工作目录逐级向上的 `.env`（最多 4 级，覆盖子模块工作目录场景）
     * 3. 可执行文件 / classpath 目录的 `.env`
     *
     * 对标原 Rust `env_loader.rs`：当前目录 → exe 目录 → exe 父目录。
     * CMP `./kotlin run` 时工作目录是项目根，直接读取 `.env`。
     *
     * @param workingDirectory 当前进程用于搜索 `.env` 的起始目录
     */
    fun loadEnvFile(
        workingDirectory: java.nio.file.Path = java.nio.file.Paths.get(".").toAbsolutePath().normalize(),
    ) {
        val candidates = mutableListOf<java.nio.file.Path>()
        // 当前工作目录及其逐级父目录（最多向上 4 级）
        var current: java.nio.file.Path? = workingDirectory
        var levels = 0
        while (current != null && levels <= 4) {
            candidates.add(current.resolve(".env"))
            current = current.parent
            levels++
        }
        // classpath 各条目目录（打包后从应用 JAR 所在目录查找）
        System.getProperty("java.class.path", "")
            .split(java.io.File.pathSeparator)
            .filter(String::isNotBlank)
            .map(java.nio.file.Paths::get)
            .mapNotNull { path -> if (java.nio.file.Files.isDirectory(path)) path else path.parent }
            .mapTo(candidates) { it.resolve(".env").normalize() }

        for (path in candidates.distinct()) {
            if (!java.nio.file.Files.exists(path)) continue
            try {
                for (line in java.nio.file.Files.readAllLines(path)) {
                    val trimmed = line.trim()
                    if (trimmed.isEmpty() || trimmed.startsWith("#")) continue
                    // 去掉可选 "export " 前缀
                    val withoutExport = trimmed.removePrefix("export ").trim()
                    val eqIdx = withoutExport.indexOf('=')
                    if (eqIdx < 0) continue
                    val key = withoutExport.substring(0, eqIdx).trim()
                    val value = cleanValue(withoutExport.substring(eqIdx + 1).trim())
                    when (key) {
                        "HWCLOUD_CLIENT_ID" -> envClientId = value
                        "HWCLOUD_CLIENT_SECRET" -> envClientSecret = value
                    }
                }
                break  // 找到第一个 .env 就返回
            } catch (e: Throwable) {
                // 读取失败忽略，继续下一个候选
            }
        }
    }

    /**
     * 去除引号
     */
    private fun cleanValue(raw: String): String {
        if (raw.length >= 2 &&
            (raw.startsWith("\"") && raw.endsWith("\"")) ||
            (raw.startsWith("'") && raw.endsWith("'"))
        ) {
            return raw.substring(1, raw.length - 1)
        }
        return raw
    }

    /**
     * 解析后的 client_id（优先级：build > env > 空）
     */
    fun resolvedClientId(): String {
        if (buildClientId.isNotBlank()) return buildClientId
        if (envClientId != null && envClientId!!.isNotBlank()) return envClientId!!
        return ""
    }

    /**
     * 解析后的 client_secret（优先级：build > env > 占位符）
     */
    fun resolvedClientSecret(): String {
        if (buildSecret.isNotBlank() && buildSecret != PLACEHOLDER_SECRET) return buildSecret
        if (envClientSecret != null && envClientSecret!!.isNotBlank() && envClientSecret != PLACEHOLDER_SECRET) return envClientSecret!!
        return PLACEHOLDER_SECRET
    }

    /**
     * 凭据是否已配置
     */
    fun clientIdConfigured(): Boolean = resolvedClientId().isNotBlank()

    /**
     * client_secret 是否已配置为非占位符的有效值。
     */
    fun clientSecretConfigured(): Boolean = resolvedClientSecret() != PLACEHOLDER_SECRET && resolvedClientSecret().isNotBlank()
}
