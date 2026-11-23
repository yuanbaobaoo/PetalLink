/**
 * Drive API —— 云盘文件操作。
 */
import { commands } from "./generated";
export { DELETE_TRACE_ERROR_PREFIX } from "./generated";
export type { DriveFile, FileCategory, FileListResult, ImportFilesResult } from "./generated";
import type { DriveFile, ImportFilesResult } from "./generated";

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
 * 列举目录全部内容（folders-first 排序）
 *
 * 单页接口最多返回 100 条，目录很大时只取第一页会让用户以为「文件不见了」；
 * 这里用后端已实现的自动翻页命令取全量。
 *
 * @param parentId - 父目录 ID，null 表示根目录
 */
export async function listFiles(parentId?: string): Promise<DriveFile[]> {
  // 自动翻页后的全部条目。
  const all = await commands.driveListAll(parentId || null);
  // folders-first 排序
  const folders = all.filter(isFolder);
  // 非目录内容保留服务端原有顺序。
  const others = all.filter((f) => !isFolder(f));
  return [...folders, ...others];
}

/**
 * 搜索文件
 *
 * @param keyword - 搜索关键词
 * @param parentId - 父目录 ID，null 表示全局搜索
 */
export async function searchFiles(keyword: string, parentId?: string): Promise<DriveFile[]> {
  // 自动翻页后的全部匹配项：宽泛关键词不会只看到前 100 条。
  const all = await commands.driveSearchAll(keyword, parentId || null);
  // 匹配的目录。
  const folders = all.filter(isFolder);
  // 匹配的普通文件。
  const others = all.filter((f) => !isFolder(f));
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

/**
 * 拖拽导入外部文件/目录到同步文件夹（复制进挂载目录后由后端自动上传）
 *
 * @param sourcePaths - 拖入的本地绝对路径数组
 * @param targetRelPath - 目标文件夹相对挂载根路径，根目录传空串
 */
export async function importLocalFiles(
  sourcePaths: string[],
  targetRelPath: string,
): Promise<ImportFilesResult> {
  // 部分失败由后端汇总在 result.failures，不在此处抛出。
  return await commands.driveImportFiles(sourcePaths, targetRelPath);
}
