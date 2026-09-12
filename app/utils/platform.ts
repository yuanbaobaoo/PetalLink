/**
 * 前端运行平台判断。
 *
 * Tauri 构建时优先使用明确的平台变量；浏览器开发和组件测试中再回退到
 * user agent。把判断集中在这里，避免设置页、首次引导与 Store 出现分歧。
 */
export function isLinuxPlatform(
  platform = import.meta.env.TAURI_ENV_PLATFORM,
  userAgent = typeof navigator !== "undefined" ? navigator.userAgent : "",
): boolean {
  if (platform) return platform.toLowerCase() === "linux";
  return /linux/i.test(userAgent);
}
