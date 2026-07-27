/**
 * Platform API —— 平台集成（开机自启等）。
 */
import { commands } from "./generated";
import { call, discard } from "./tauri";

/**
 * 在 Finder 中打开路径
 */
export function openInFinder(path: string): Promise<boolean> {
  return call(commands.openInFinder(path));
}

/**
 * 获取开机自启状态
 */
export function launchAtLoginIsEnabled(): Promise<boolean> {
  return call(commands.launchAtLoginIsEnabled());
}

/**
 * 设置开机自启
 */
export function launchAtLoginSetEnabled(enabled: boolean): Promise<boolean> {
  return call(commands.launchAtLoginSetEnabled(enabled));
}

/**
 * 切换托盘（菜单栏）图标显示，持久化并立即生效
 */
export function traySetVisible(visible: boolean): Promise<void> {
  return discard(commands.traySetVisible(visible));
}

/**
 * 查询托盘图标当前实际可见性（运行时真实状态）
 */
export function trayIsVisible(): Promise<boolean> {
  return call(commands.trayIsVisible());
}

/**
 * 获取应用版本号（读取 Cargo.toml 编译期注入的版本）
 */
export function getAppVersion(): Promise<string> {
  return call(commands.appGetVersion());
}
