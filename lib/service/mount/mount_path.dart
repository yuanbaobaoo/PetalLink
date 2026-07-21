/// 挂载目录路径安全工具（静态工具类）。
///
/// 严格对齐 Rust 原版 `src/core/paths.rs`：
/// - [validateRelativePath] 拒绝绝对路径、上跳、空段、反斜杠与 NUL
/// - [safeJoinUnder] 校验后拼接到挂载根下
/// - [relativePathFromMount] 绝对路径 → 挂载根下安全相对路径
library;

import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_error.dart';

/// 挂载目录路径工具。
class MountPath {
  MountPath._();

  /// 校验单个路径片段，拒绝空值、路径分隔符和特殊目录。
  static void validatePathSegment(String segment) {
    if (segment.isEmpty ||
        segment == '.' ||
        segment == '..' ||
        segment.contains('/') ||
        segment.contains('\\') ||
        segment.contains('\x00')) {
      throw AppError.config('路径片段不安全：$segment');
    }
  }

  /// 校验挂载目录内使用的相对路径，拒绝绝对路径、上跳、空段和反斜杠。
  static void validateRelativePath(String relPath, {bool allowEmpty = false}) {
    if (relPath.isEmpty) {
      if (allowEmpty) return;
      throw AppError.config('相对路径不能为空');
    }
    if (relPath.contains('\\') ||
        relPath.contains('\x00') ||
        relPath.contains('//')) {
      throw AppError.config('相对路径不安全：$relPath');
    }
    if (p.isAbsolute(relPath)) {
      throw AppError.config('相对路径不能是绝对路径：$relPath');
    }
    for (final segment in relPath.split('/')) {
      validatePathSegment(segment);
    }
  }

  /// 在校验后把相对路径拼到指定根目录下。
  static String safeJoinUnder(String base, String relPath,
      {bool allowEmpty = false}) {
    validateRelativePath(relPath, allowEmpty: allowEmpty);
    return p.join(base, relPath);
  }

  /// 把前端给出的绝对路径转换为挂载根下的安全相对路径。
  static String relativePathFromMount(String mountDir, String candidate) {
    if (!p.isAbsolute(candidate)) {
      throw AppError.config('本地路径必须是绝对路径：$candidate');
    }
    final rel = p.relative(candidate, from: mountDir);
    if (rel == '.' || rel == '..' || rel.startsWith('../')) {
      throw AppError.config('路径不在同步目录内：$candidate');
    }
    validateRelativePath(rel);
    return rel;
  }
}
