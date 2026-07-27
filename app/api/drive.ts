/**
 * Drive API —— 云盘文件操作。
 */
import { commands } from "./generated";
export { DELETE_TRACE_ERROR_PREFIX } from "./generated";
export type { DriveFile, FileCategory, FileListResult } from "./generated";
import type { DriveFile } from "./generated";

/**
 * 是否文件夹（大小写不敏感，兼容后端返回 "Folder" / "folder"）
 *
 * @param f - 文件对象
 */
export function isFolder(f: DriveFile): boolean {
  return (f.category ?? "").toLowerCase() === "folder";
}

/**
 * 文件类型图标（返回 icon-name，配合 <MateIcon :name="..."> 使用）
 *
 * @param f - 文件对象
 */
export function fileTypeIcon(f: DriveFile): string {
  if (isFolder(f)) return "folder";
  // 服务端类别统一转为小写匹配图标。
  const cat = (f.category ?? "").toLowerCase();
  switch (cat) {
    case "image": return "image";
    case "video": return "video";
    case "audio": return "file";
    case "document": return "file-text";
    case "archive": return "archive";
    case "package": return "archive";
    case "executable": return "settings";
    default: return "file";
  }
}

/**
 * 列举目录内容（folders-first 排序）
 *
 * @param parentId - 父目录 ID，null 表示根目录
 */
export async function listFiles(parentId?: string): Promise<DriveFile[]> {
  // 首屏列表结果。
  const result = await commands.driveList(parentId || null, null, null);
  // folders-first 排序
  const folders = result.files.filter(isFolder);
  // 非目录内容保留服务端原有顺序。
  const others = result.files.filter((f) => !isFolder(f));
  return [...folders, ...others];
}

/**
 * 搜索文件
 *
 * @param keyword - 搜索关键词
 * @param parentId - 父目录 ID，null 表示全局搜索
 */
export async function searchFiles(keyword: string, parentId?: string): Promise<DriveFile[]> {
  // 搜索结果仍按目录优先展示。
  const result = await commands.driveSearch(keyword, parentId || null, null);
  // 匹配的目录。
  const folders = result.files.filter(isFolder);
  // 匹配的普通文件。
  const others = result.files.filter((f) => !isFolder(f));
  return [...folders, ...others];
}

/**
 * 获取缩略图（返回 base64 data URL）
 *
 * @param fileId - 文件 ID
 */
export async function getThumbnail(fileId: string): Promise<string | null> {
  try {
    // 后端已保留或识别真实图片 MIME 的 data URL
    const dataUrl = await commands.driveGetThumbnail(fileId);
    return dataUrl.startsWith("data:image/") ? dataUrl : null;
  } catch {
    return null;
  }
}
