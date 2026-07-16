package io.github.yuanbaobaao.petallink.config

import java.nio.file.Files
import java.nio.file.Paths
import java.nio.file.StandardOpenOption

/**
 * JVM 配置持久化实现（actual）。
 * 用 java.nio.file 读写 JSON，权限由文件系统管理。
 */
actual class ConfigStore actual constructor() {
    private val json = kotlinx.serialization.json.Json {
        prettyPrint = true; ignoreUnknownKeys = true
    }

    private val configPath: java.nio.file.Path
        get() = Paths.get(System.getProperty("user.home"), "Library", "Application Support", "PetalLink", "config.json")

    actual fun load(): UserConfig? {
        return try {
            val path = configPath
            if (!Files.exists(path)) return null
            val text = Files.readString(path)
            if (text.isBlank()) null else json.decodeFromString(UserConfig.serializer(), text)
        } catch (e: Throwable) {
            null
        }
    }

    actual fun save(config: UserConfig) {
        try {
            val path = configPath
            Files.createDirectories(path.parent)
            val text = json.encodeToString(UserConfig.serializer(), config)
            Files.write(path, text.toByteArray(), StandardOpenOption.CREATE, StandardOpenOption.TRUNCATE_EXISTING)
        } catch (e: Throwable) {
            // 写入失败忽略
        }
    }
}
