package io.github.yuanbaobaoo.petallink.config

/** 配置持久化边界。路径由 Composition Root 注入，测试不得依赖真实用户目录。 */
interface ConfigStore {
    /** 读取配置；不存在返回 null */
    fun load(): UserConfig?

    /** 保存配置 */
    fun save(config: UserConfig)
}
