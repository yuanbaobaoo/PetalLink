package io.github.yuanbaobaao.petallink.config

/**
 * 配置持久化接口（expect，macosMain 提供 actual）。
 *
 * actual 用 JSON 文件持久化（kotlinx-serialization），路径经 cache_paths 规则。
 */
expect class ConfigStore() {
    /** 读取配置；不存在返回 null */
    fun load(): UserConfig?

    /** 保存配置 */
    fun save(config: UserConfig)
}
