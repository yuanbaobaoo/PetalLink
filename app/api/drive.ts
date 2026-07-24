/**
 * Drive API —— 云盘文件操作。
 */
import { invoke } from "./tauri";

/** 后端 DriveFile 结构 */
export interface DriveFile {
  id: string;
  name: string;
  category: string;
  size: number;
  parent_folder?: string[];
  description?: string;
  created_time?: string;
  edited_time?: string;
  mime_type?: string;
  content_hash?: string;
  thumbnail_link?: string;
}

/** 后端 FileListResult 结构 */
export interface FileListResult {
  files: DriveFile[];
  next_cursor?: string;
}

/** 后端 DriveAbout 结构 */
export interface DriveAbout {
  user_capacity: number;
  used_space: number;
  user_display_name?: string;
}

/** 文件分类 */
export type FileCategory =
  | "Folder" | "Audio" | "Video" | "Image" | "Document"
  | "Package" | "Archive" | "Executable" | "None";

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
  const result = await invoke<FileListResult>("drive_list", {
    parentId: parentId || null,
  });
  // folders-first 排序
  const folders = result.files.filter(isFolder);
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
  const result = await invoke<FileListResult>("drive_search", {
    keyword,
    parentId: parentId || null,
  });
  const folders = result.files.filter(isFolder);
  const others = result.files.filter((f) => !isFolder(f));
  return [...folders, ...others];
}

/**
 * 创建文件夹
 *
 * @param name - 文件夹名称
 * @param parentId - 父目录 ID
 */
export function createFolder(name: string, parentId?: string): Promise<DriveFile> {
  return invoke<DriveFile>("drive_create_folder", { name, parentId: parentId || null });
}

// 留痕失败错误标识符（与后端 src/commands/drive.rs 的 DELETE_TRACE_ERROR_PREFIX 完全一致），
// 前端据此区分「文件未删」与「文件已删但记录未写入」。改动任一侧必须同步另一侧。
export const DELETE_TRACE_ERROR_PREFIX = "TRACE_FAILED:";

/**
 * 删除文件（软删除进回收站）
 *
 * @param id - 文件 ID
 * @param name - 文件名（无本地基线时用于传输队列留痕显示，可选）
 */
export function deleteFile(id: string, name?: string): Promise<void> {
  return invoke<void>("drive_delete_file", { id, name });
}

/**
 * 重命名文件
 *
 * @param id - 文件 ID
 * @param newName - 新名称
 */
export function renameFile(id: string, newName: string): Promise<DriveFile> {
  return invoke<DriveFile>("drive_rename_file", { id, newName });
}

/**
 * 获取缩略图（返回 base64 data URL）
 *
 * @param fileId - 文件 ID
 */
export async function getThumbnail(fileId: string): Promise<string | null> {
  try {
    // 后端已保留或识别真实图片 MIME 的 data URL
    const dataUrl = await invoke<string>("drive_get_thumbnail", { fileId });
    return dataUrl.startsWith("data:image/") ? dataUrl : null;
  } catch {
    return null;
  }
}

/**
 * 获取配额信息
 */
export function getAbout(): Promise<DriveAbout> {
  return invoke<DriveAbout>("drive_get_about");
}

/**
 * 下载文件到本地路径
 *
 * @param fileId - 文件 ID
 * @param destPath - 目标本地路径
 */
export function downloadFile(fileId: string, destPath: string): Promise<void> {
  return invoke<void>("drive_download_file", { fileId, destPath });
}

/**
 * 上传本地文件到云端
 *
 * @param localPath - 本地文件路径
 * @param parentId - 目标父目录 ID
 */
export function uploadFile(localPath: string, parentId?: string): Promise<DriveFile> {
  return invoke<DriveFile>("drive_upload_file", { localPath, parentId: parentId || null });
}
