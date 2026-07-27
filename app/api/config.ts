/**
 * Config API —— 配置读写。
 */
import { commands } from "./generated";
import { call, discard } from "./tauri";
import type { AppConfig as GeneratedAppConfig } from "./generated";

/**
 * 后端序列化始终返回完整配置；Rust 的 `serde(default)` 仅让导入旧配置时兼容缺字段。
 */
export type AppConfig = Required<GeneratedAppConfig>;

/**
 * 加载配置
 */
export function loadConfig(): Promise<AppConfig> {
  return call(commands.configLoad()) as Promise<AppConfig>;
}

/**
 * 保存配置
 */
export function saveConfig(config: AppConfig): Promise<void> {
  return discard(commands.configSave(config));
}

/**
 * 导出配置 JSON
 */
export function exportConfigJson(): Promise<string> {
  return call(commands.configExportJson());
}

/**
 * 导入配置 JSON
 */
export function importConfigJson(jsonStr: string): Promise<AppConfig> {
  return call(commands.configImportJson(jsonStr)) as Promise<AppConfig>;
}

/**
 * 清空全部缓存（退出登录态+DB+缓存+配置）
 */
export function clearCache(): Promise<void> {
  return discard(commands.appClearCache());
}
