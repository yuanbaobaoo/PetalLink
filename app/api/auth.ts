/**
 * Auth API —— 封装后端 auth 命令。
 */
import { commands } from "./generated";
import { call, discard } from "./tauri";
export type { AuthState, TokenPair, UserInfo } from "./generated";
import type { AuthState, TokenPair, UserInfo } from "./generated";

// 匿名账号的显示名称
const ANONYMOUS_LABEL = "匿名账号";

/**
 * 用户主要展示名（对齐后端 primary_label 逻辑）
 */
export function primaryLabel(u?: UserInfo | null): string | null {
  if (!u) return null;
  // 空白身份字段不参与展示名回退。
  const ne = (s?: string | null) => s?.trim() || null;
  return (
    ne(u.display_name) ||
    ne(u.mobile) ||
    ne(u.name) ||
    ne(u.nickname) ||
    ne(u.open_id) ||
    ne(u.sub) ||
    null
  );
}

/**
 * 用户副标题（对齐后端 secondary_label）
 */
export function secondaryLabel(u?: UserInfo | null): string | null {
  if (!u) return null;
  // 主展示名用于排除重复副标题。
  const pri = primaryLabel(u);
  // 空白身份字段不参与副标题选择。
  const ne = (s?: string | null) => s?.trim() || null;
  // 邮箱优先作为副标题。
  const email = ne(u.email);
  if (email && email !== pri) return email;
  // 手机号作为邮箱缺失时的回退。
  const mobile = ne(u.mobile);
  if (mobile && mobile !== pri) return mobile;
  if (u.is_anonymized) return ANONYMOUS_LABEL;
  return null;
}

/**
 * 头像首字符
 */
export function initial(u?: UserInfo | null): string | null {
  // 头像字符来自最终展示名，保证两处身份一致。
  const label = primaryLabel(u);
  if (!label) return null;
  // 取第一个 Unicode 字符（CJK 安全）
  return Array.from(label)[0] ?? null;
}

/**
 * 检查 client_secret 是否已配置
 */
export function checkSecret(): Promise<boolean> {
  return call(commands.authCheckSecret());
}

/**
 * 启动时恢复登录态
 */
export function restore(): Promise<AuthState> {
  return call(commands.authRestore());
}

/**
 * 发起 OAuth 登录
 */
export function login(port: number): Promise<TokenPair> {
  return call(commands.authLogin(port));
}

/**
 * 取消正在进行的授权
 */
export function cancelLogin(): Promise<void> {
  return discard(commands.authCancelLogin());
}

/**
 * 退出登录
 */
export function logout(): Promise<void> {
  return discard(commands.authLogout());
}

/**
 * 拉取当前用户信息
 */
export function getUserInfo(): Promise<UserInfo> {
  return call(commands.authGetUserInfo());
}

/**
 * 检查是否已登录
 */
export function isLoggedIn(): Promise<boolean> {
  return call(commands.authIsLoggedIn());
}
