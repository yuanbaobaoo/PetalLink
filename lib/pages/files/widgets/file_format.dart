/// 文件页共享格式化工具（对标 CMP FileListScreen.kt 顶层 formatFileSize）。
library;

/// 文件大小格式化（对标原 Vue formatFileSize）。
String formatFileSize(int bytes) {
  if (bytes < 1024) return '$bytes B';
  if (bytes < 1048576) return '${(bytes / 1024).toStringAsFixed(1)} KB';
  if (bytes < 1073741824) return '${(bytes / 1048576).toStringAsFixed(1)} MB';
  return '${(bytes / 1073741824).toStringAsFixed(2)} GB';
}
