/**
 * 文件系统工具 —— 目录空检查等本地 FS 辅助逻辑。
 *
 * 统一 `SyncSetupBanner.vue` 与 `SettingsPage.vue` 中重复的空目录判定。
 */

import { readDir } from "@tauri-apps/plugin-fs";

/**
 * 同步目录空判定的跳过模式（与后端 DEFAULT_SKIP_PATTERNS 口径一致）。
 *
 * 这些文件/模式即使存在也视为"空目录"：隐藏文件（`.` 开头）、
 * 临时文件、Office 锁定文件、系统回收站。
 */
const SKIP_PATTERNS = [".DS_Store", ".tmp", "~$*", ".Trash"];

/**
 * 判断目录是否为"空"（过滤隐藏文件 + skipPatterns 后无可见文件）。
 *
 * 用于同步目录选择校验：必须空目录才允许作为同步目录，避免与已有文件冲突。
 *
 * @param dir - 目录绝对路径
 * @returns true 表示目录可视为空
 */
export async function isEmptyDir(dir: string): Promise<boolean> {
  const entries = await readDir(dir);
  const visible = entries.filter((e) => {
    const name = e.name ?? "";
    if (!name) return false;
    if (name.startsWith(".")) return false; // 隐藏文件
    for (const p of SKIP_PATTERNS) {
      if (p.includes("*")) {
        if (new RegExp("^" + p.replace(/\./g, "\\.").replace(/\*/g, ".*")).test(name)) return false;
      } else if (name === p) {
        return false;
      }
    }
    return true;
  });
  return visible.length === 0;
}
