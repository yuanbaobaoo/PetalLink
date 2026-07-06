/**
 * 格式化工具 —— 文件大小 / 日期时间 / 数字补零。
 *
 * 统一各视图组件中重复的格式化逻辑，保证口径一致。
 */

/**
 * 数字补零到 2 位。
 *
 * @param n - 待补零的数字
 */
export function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/**
 * 格式化文件大小（字节 → 自动适配单位 B/KB/MB/GB/TB）。
 *
 * 0 或 falsy 返回 "—"（占位显示），与原各处 fmtSize/formatSize 行为一致。
 *
 * @param bytes - 字节数
 */
export function formatFileSize(bytes: number): string {
  if (!bytes) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), u.length - 1);
  return `${(bytes / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 1)} ${u[i]}`;
}

/**
 * 格式化日期时间。
 *
 * @param input - ISO 字符串或毫秒时间戳
 * @param withSeconds - 是否包含秒（默认 false，即 YYYY-MM-DD HH:mm）
 * @returns 格式化字符串；input 为空时返回 "—"
 */
export function formatDateTime(input: string | number | null | undefined, withSeconds = false): string {
  if (!input) return "—";
  const d = new Date(input);
  const base = `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  return withSeconds ? `${base}:${pad2(d.getSeconds())}` : base;
}
