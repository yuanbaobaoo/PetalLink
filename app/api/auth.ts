/**
 * Auth 展示适配。
 */
export type { UserInfo } from "./generated";
import type { UserInfo } from "./generated";

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
