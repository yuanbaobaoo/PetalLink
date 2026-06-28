/**
 * Platform API —— 平台集成（开机自启等）。
 */
import { invoke } from "./tauri";

/** 在 Finder 中打开路径 */
export function openInFinder(path: string): Promise<boolean> {
  return invoke<boolean>("open_in_finder", { path });
}

/** 获取开机自启状态 */
export function launchAtLoginIsEnabled(): Promise<boolean> {
  return invoke<boolean>("launch_at_login_is_enabled");
}

/** 设置开机自启 */
export function launchAtLoginSetEnabled(enabled: boolean): Promise<boolean> {
  return invoke<boolean>("launch_at_login_set_enabled", { enabled });
}

/** 获取应用版本号（读取 Cargo.toml 编译期注入的版本） */
export function getAppVersion(): Promise<string> {
  return invoke<string>("app_get_version");
}
