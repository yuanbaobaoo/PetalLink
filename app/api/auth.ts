/**
 * Auth API —— 封装后端 auth 命令。
 */
import { invoke } from "./tauri";

// 匿名账号的显示名称
const ANONYMOUS_LABEL = "匿名账号";

/** 后端 TokenPair 结构 */
export interface TokenPair {
  access_token: string;
  refresh_token: string;
  expires_at: number;
  token_type: string;
  scope?: string;
}

/** 后端 AuthState 结构 */
export interface AuthState {
  logged_in: boolean;
  secret_configured: boolean;
  callback_port: number;
}

/** 后端 UserInfo 结构 */
export interface UserInfo {
  sub?: string;
  open_id?: string;
  union_id?: string;
  display_name?: string;
  name?: string;
  nickname?: string;
  email?: string;
  mobile?: string;
  avatar_url?: string;
  is_anonymized: boolean;
}

/** 用户主要展示名（对齐后端 primary_label 逻辑） */
export function primaryLabel(u?: UserInfo | null): string | null {
  if (!u) return null;
  const ne = (s?: string) => s?.trim() || null;
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

/** 用户副标题（对齐后端 secondary_label） */
export function secondaryLabel(u?: UserInfo | null): string | null {
  if (!u) return null;
  const pri = primaryLabel(u);
  const ne = (s?: string) => s?.trim() || null;
  const email = ne(u.email);
  if (email && email !== pri) return email;
  const mobile = ne(u.mobile);
  if (mobile && mobile !== pri) return mobile;
  if (u.is_anonymized) return ANONYMOUS_LABEL;
  return null;
}

/** 头像首字符 */
export function initial(u?: UserInfo | null): string | null {
  const label = primaryLabel(u);
  if (!label) return null;
  // 取第一个 Unicode 字符（CJK 安全）
  return Array.from(label)[0] ?? null;
}

/** 检查 client_secret 是否已配置 */
export function checkSecret(): Promise<boolean> {
  return invoke<boolean>("auth_check_secret");
}

/** 启动时恢复登录态 */
export function restore(): Promise<AuthState> {
  return invoke<AuthState>("auth_restore");
}

/** 发起 OAuth 登录 */
export function login(port: number): Promise<TokenPair> {
  return invoke<TokenPair>("auth_login", { port });
}

/** 取消正在进行的授权 */
export function cancelLogin(): Promise<void> {
  return invoke<void>("auth_cancel_login");
}

/** 退出登录 */
export function logout(): Promise<void> {
  return invoke<void>("auth_logout");
}

/** 拉取当前用户信息 */
export function getUserInfo(): Promise<UserInfo> {
  return invoke<UserInfo>("auth_get_user_info");
}

/** 检查是否已登录 */
export function isLoggedIn(): Promise<boolean> {
  return invoke<boolean>("auth_is_logged_in");
}
