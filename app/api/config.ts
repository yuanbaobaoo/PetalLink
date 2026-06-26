/**
 * Config API —— 配置读写。
 */
import { invoke } from "./tauri";

/** 后端 AppConfig 结构 */
export interface AppConfig {
  oauth_redirect_uri: string;
  oauth_callback_port: number;
  mount_dir: string;
  mount_configured: boolean;
  concurrency: number;
  poll_interval_sec: number;
  debounce_sec: number;
  skip_patterns: string[];
  sort_field: string;
  sort_order: string;
}

/** 加载配置 */
export function loadConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("config_load");
}

/** 保存配置 */
export function saveConfig(config: AppConfig): Promise<void> {
  return invoke<void>("config_save", { config });
}

/** 导出配置 JSON */
export function exportConfigJson(): Promise<string> {
  return invoke<string>("config_export_json");
}

/** 导入配置 JSON */
export function importConfigJson(jsonStr: string): Promise<AppConfig> {
  return invoke<AppConfig>("config_import_json", { jsonStr });
}

/** 清空全部缓存（退出登录态+DB+缓存+配置） */
export function clearCache(): Promise<void> {
  return invoke<void>("app_clear_cache");
}
