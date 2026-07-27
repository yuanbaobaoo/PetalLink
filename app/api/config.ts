import { commands } from "./generated";
import type { AppConfig as GeneratedAppConfig } from "./generated";

/**
 * 后端序列化始终返回完整配置；Rust 的 `serde(default)` 仅让导入旧配置时兼容缺字段。
 */
export type AppConfig = Required<GeneratedAppConfig>;

/**
 * 加载后端保证字段完整的当前配置。
 */
export async function loadConfig(): Promise<AppConfig> {
  return await commands.configLoad() as AppConfig;
}

/**
 * 解析后端补齐默认值后的导入配置。
 *
 * @param json - 待校验的配置 JSON
 */
export async function importConfigJson(json: string): Promise<AppConfig> {
  return await commands.configImportJson(json) as AppConfig;
}
