/// 文件跳过规则 —— 内部文件硬编码过滤 + 用户配置 glob。
///
/// 严格对齐 Rust 原版 `src/mount/skip.rs`：
/// 四处硬编码过滤（v1.8 全局过滤，无论用户如何配置 skipPatterns）：
/// 1. `.hwcloud_` 前缀（内部缓存/快照文件）
/// 2. `.hwcloud_placeholder` 后缀（旧版占位符）
/// 3. `.tmp` 后缀（下载原子写临时文件）
/// 4. 用户配置的 skipPatterns（简化 glob）
library;

/// 挂载层跳过规则（静态工具类）。
class MountSkip {
  MountSkip._();

  /// 内部文件前缀（对齐 Rust constants::INTERNAL_FILE_PREFIX）
  static const String internalFilePrefix = '.hwcloud_';

  /// 下载原子写临时文件后缀（对齐 Rust constants::TMP_SUFFIX）
  static const String tmpSuffix = '.tmp';

  /// 旧版占位符后缀（对齐 Rust LEGACY_PLACEHOLDER_SUFFIX，仅用于清理遗留文件）
  static const String legacyPlaceholderSuffix = '.hwcloud_placeholder';

  /// 默认用户跳过模式（glob）。
  ///
  /// `.tmp` 已被硬编码规则 3 覆盖，保留在用户模式中与旧版配置展示一致。
  static const List<String> defaultPatterns = [
    '.DS_Store',
    '.tmp',
    '~\$*',
    '.Trash',
  ];

  /// 判断文件名是否应被跳过（不参与同步）。
  ///
  /// 统一逻辑，供 scanLocal / localWatcher / syncEngine 复用。
  static bool shouldSkip(String name, List<String> skipPatterns) {
    // 1. .hwcloud_ 前缀（内部文件，硬编码全局过滤）
    if (name.startsWith(internalFilePrefix)) {
      return true;
    }
    // 2. 旧版占位符后缀
    if (name.endsWith(legacyPlaceholderSuffix)) {
      return true;
    }
    // 3. .tmp 后缀（下载原子写临时文件）
    if (name.endsWith(tmpSuffix)) {
      return true;
    }
    // 4. 用户配置的 skipPatterns（简化 glob 匹配）
    for (final pattern in skipPatterns) {
      if (globMatches(pattern, name)) {
        return true;
      }
    }
    return false;
  }

  /// 判断规范相对路径中是否包含任一应跳过的目录或文件名。
  static bool shouldSkipRelativePath(
      String relativePath, List<String> skipPatterns) {
    return relativePath
        .split('/')
        .where((segment) => segment.isNotEmpty)
        .any((segment) => shouldSkip(segment, skipPatterns));
  }

  /// 简化 glob 匹配（对齐 Rust `glob_matches`）。
  ///
  /// `*` → `.*`，`?` → `.`，转义 `\ . + ( ) [ ] { } ^ $ |`，全匹配。
  static bool globMatches(String pattern, String name) {
    final buf = StringBuffer('^');
    // 按 Unicode 标量值遍历（对齐 Rust chars()），避免代理对被拆开
    for (final rune in pattern.runes) {
      final c = String.fromCharCode(rune);
      switch (c) {
        case '*':
          buf.write('.*');
        case '?':
          buf.write('.');
        case '\\' ||
              '.' ||
              '+' ||
              '(' ||
              ')' ||
              '[' ||
              ']' ||
              '{' ||
              '}' ||
              '^' ||
              '\$' ||
              '|':
          buf.write('\\');
          buf.write(c);
        default:
          buf.write(c);
      }
    }
    buf.write('\$');
    final RegExp re;
    try {
      re = RegExp(buf.toString());
    } catch (_) {
      return false;
    }
    return re.hasMatch(name);
  }
}
