package io.github.yuanbaobaao.petallink.config

import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * JVM JSON 配置持久化实现。读写错误向调用方传播，保存采用同目录临时文件替换。
 */
class JsonConfigStore(private val configPath: Path) : ConfigStore {
    private val json = kotlinx.serialization.json.Json {
        prettyPrint = true; ignoreUnknownKeys = true
    }

    override fun load(): UserConfig? {
        if (!Files.exists(configPath)) return null
        val text = Files.readString(configPath)
        if (text.isBlank()) return null
        val (config, dirty) = decodeAndMigrate(text)
        validate(config)
        if (dirty) save(config)
        return config
    }

    override fun save(config: UserConfig) {
        validate(config)
        validateMountAccess(config)
        Files.createDirectories(configPath.parent)
        val temp = Files.createTempFile(configPath.parent, "config-", ".tmp")
        try {
            val text = json.encodeToString(UserConfig.serializer(), config)
            Files.writeString(temp, text)
            try {
                Files.move(temp, configPath, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            } catch (_: java.nio.file.AtomicMoveNotSupportedException) {
                Files.move(temp, configPath, StandardCopyOption.REPLACE_EXISTING)
            }
        } finally {
            Files.deleteIfExists(temp)
        }
    }

    /** 导入使用与 load 相同的旧值迁移与校验，但由命令层统一决定何时落盘。 */
    fun parseImport(text: String): UserConfig = decodeAndMigrate(text).first.also(::validate)

    private fun decodeAndMigrate(text: String): Pair<UserConfig, Boolean> {
        val parsed = json.parseToJsonElement(text).jsonObject
        var dirty = false
        val normalized = buildJsonObject {
            parsed.forEach { (key, value) -> put(key, value) }
            val poll = parsed["pollIntervalSec"]?.jsonPrimitive?.content?.toLongOrNull()
            if (poll != null && poll != 0L && poll < ConfigValidator.MIN_POLL_INTERVAL_SEC) {
                put("pollIntervalSec", JsonPrimitive(UserConfig().pollIntervalSec)); dirty = true
            }
            val debounce = parsed["debounceSec"]?.jsonPrimitive?.content?.toLongOrNull()
            if (debounce == 30L) {
                put("debounceSec", JsonPrimitive(UserConfig().debounceSec)); dirty = true
            }
            val configured = parsed["mountConfigured"]?.jsonPrimitive?.content?.toBooleanStrictOrNull() ?: false
            if (!configured && parsed["mountDir"]?.jsonPrimitive?.content == "~/hwcloud-drive") {
                put("mountDir", JsonPrimitive("")); dirty = true
            }
        }
        return json.decodeFromJsonElement(UserConfig.serializer(), normalized) to dirty
    }

    private fun validate(config: UserConfig) {
        val errors = ConfigValidator.validate(config)
        require(errors.isEmpty()) { errors.joinToString("；") }
    }

    private fun validateMountAccess(config: UserConfig) {
        if (!config.mountConfigured) return
        val expanded = JvmMountPaths.resolve(config.mountDir)
        val home = Path.of(System.getProperty("user.home")).toAbsolutePath().normalize()
        require(expanded != home) { "mountDir 不能是用户主目录" }
        if (Files.exists(expanded) && !Files.isDirectory(expanded)) {
            throw IllegalArgumentException("同步目录不是文件夹：$expanded")
        }
        Files.createDirectories(expanded)
        val probe = Files.createTempFile(expanded, ".petallink-write-test-", null)
        Files.delete(probe)
    }
}
