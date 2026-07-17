package io.github.yuanbaobaoo.petallink.config

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * ConfigValidator 单测（对标 src/core/config.rs 校验规则）。
 */
class ConfigValidatorTest {

    private val valid = UserConfig(mountDir = "/Users/test/PetalLink", mountConfigured = true)

    @Test
    fun 合法配置无错误() {
        assertTrue(ConfigValidator.isValid(valid))
    }

    @Test
    fun concurrency为零失败() {
        val errors = ConfigValidator.validate(valid.copy(concurrency = 0))
        assertTrue(errors.any { it.contains("concurrency") })
    }

    @Test
    fun concurrency超过20失败() {
        val errors = ConfigValidator.validate(valid.copy(concurrency = 21))
        assertTrue(errors.any { it.contains("concurrency") })
    }

    @Test
    fun concurrency为6通过() {
        assertTrue(ConfigValidator.isValid(valid.copy(concurrency = 6)))
    }

    @Test
    fun pollIntervalSec小于60失败() {
        val errors = ConfigValidator.validate(valid.copy(pollIntervalSec = 30))
        assertTrue(errors.any { it.contains("pollIntervalSec") })
    }

    @Test
    fun pollIntervalSec为零通过_表示禁用() {
        assertTrue(ConfigValidator.isValid(valid.copy(pollIntervalSec = 0)))
    }

    @Test
    fun debounceSec为零失败() {
        val errors = ConfigValidator.validate(valid.copy(debounceSec = 0))
        assertTrue(errors.any { it.contains("debounceSec") })
    }

    @Test
    fun oauthCallbackPort为零失败() {
        val errors = ConfigValidator.validate(valid.copy(oauthCallbackPort = 0))
        assertTrue(errors.any { it.contains("oauthCallbackPort") })
    }

    @Test
    fun mountDir为空且已配置时失败() {
        val errors = ConfigValidator.validate(valid.copy(mountDir = ""))
        assertTrue(errors.any { it.contains("mountDir") })
    }

    @Test
    fun 首次启动未配置目录时通过() {
        assertTrue(ConfigValidator.isValid(UserConfig()))
    }

    @Test
    fun mountDir为根目录失败() {
        val errors = ConfigValidator.validate(valid.copy(mountDir = "/"))
        assertTrue(errors.any { it.contains("mountDir") })
    }

    @Test
    fun mountDir含双点失败() {
        val errors = ConfigValidator.validate(valid.copy(mountDir = "/Users/../etc"))
        assertTrue(errors.any { it.contains("mountDir") })
    }

    @Test
    fun mountDir支持以主目录为基准的缩写() {
        assertTrue(ConfigValidator.isValid(valid.copy(mountDir = "~/PetalLink")))
    }
}
