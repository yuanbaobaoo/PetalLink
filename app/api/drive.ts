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

/** 是否文件夹（大小写不敏感，兼容后端返回 "Folder" / "folder"） */
export function isFolder(f: DriveFile): boolean {
  return (f.category ?? "").toLowerCase() === "folder";
}

/** 文件类型图标（返回 icon-name，配合 <MateIcon :name="..."> 使用） */
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

/** 列举目录内容（folders-first 排序） */
export async function listFiles(parentId?: string): Promise<DriveFile[]> {
  const result = await invoke<FileListResult>("drive_list", {
    parentId: parentId || null,
  });
  // folders-first 排序
  const folders = result.files.filter(isFolder);
  const others = result.files.filter((f) => !isFolder(f));
  return [...folders, ...others];
}

/** 搜索文件 */
export async function searchFiles(keyword: string, parentId?: string): Promise<DriveFile[]> {
  const result = await invoke<FileListResult>("drive_search", {
    keyword,
    parentId: parentId || null,
  });
  const folders = result.files.filter(isFolder);
  const others = result.files.filter((f) => !isFolder(f));
  return [...folders, ...others];
}

/** 创建文件夹 */
export function createFolder(name: string, parentId?: string): Promise<DriveFile> {
  return invoke<DriveFile>("drive_create_folder", { name, parentId: parentId || null });
}

/** 删除文件（软删除进回收站） */
export function deleteFile(id: string): Promise<void> {
  return invoke<void>("drive_delete_file", { id });
}

/** 重命名文件 */
export function renameFile(id: string, newName: string): Promise<DriveFile> {
  return invoke<DriveFile>("drive_rename_file", { id, newName });
}

/** 获取缩略图（返回 base64 data URL） */
export async function getThumbnail(fileId: string): Promise<string | null> {
  try {
    const bytes = await invoke<number[]>("drive_get_thumbnail", { fileId });
    if (!bytes || bytes.length === 0) return null;
    // 转 base64 data URL
    const binary = bytes.map((b) => String.fromCharCode(b)).join("");
    return `data:image/png;base64,${btoa(binary)}`;
  } catch {
    return null;
  }
}

/** 获取配额信息 */
export function getAbout(): Promise<DriveAbout> {
  return invoke<DriveAbout>("drive_get_about");
}

/** 下载文件 */
export function downloadFile(fileId: string, destPath: string): Promise<void> {
  return invoke<void>("drive_download_file", { fileId, destPath });
}

/** 上传文件 */
export function uploadFile(localPath: string, parentId?: string): Promise<DriveFile> {
  return invoke<DriveFile>("drive_upload_file", { localPath, parentId: parentId || null });
}
